use crate::{Error, Execution, NodeDatabase, Progress, WriteGuard, WritePoint};
use ora_node_protocol::ExecutionId;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf, sync::Arc};

/// Names one dispatch of a Git command. The same value names the attempt's lock file, so a
/// restarted Node can ask whether the process tree it started is gone.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttemptId(String);

impl AttemptId {
    /// Draws a fresh identity; one is never reused for another dispatch.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Adopts an identity read back from a file name or a row, which must be a UUID.
    pub fn parse(value: &str) -> Option<Self> {
        uuid::Uuid::parse_str(value)
            .ok()
            .map(|id| Self(id.to_string()))
    }

    /// The textual form used as the row key and the lock file stem.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AttemptId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Node-owned association persisted before a Git side effect is dispatched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessAttempt {
    pub execution: ExecutionId,
    pub id: AttemptId,
}

/// What an attempt ran, kept for whoever inspects an attempt that never settled. It is never
/// executed again and carries no environment, which may hold deployment secrets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptCommand {
    pub cwd: PathBuf,
    pub args: Vec<String>,
}

/// The single way a recorded attempt ended. The three are mutually exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptEnding {
    /// Git exited with its own verdict.
    Exited(i32),
    /// A signal ended Git before it reached a verdict.
    Terminated(i32),
    /// The process tree is gone but nobody collected its exit status: a restarted Node found the
    /// attempt stopped. It can back an interruption, never a success or a Git failure.
    Stopped,
}

/// A narrow second connection shares the original database lease, not a second Node authority.
/// Its transactions finish before a command is dispatched or business-state transactions begin.
pub struct ProcessJournal<G: WriteGuard> {
    connection: Connection,
    guard: Arc<G>,
    _lease: Arc<ora_utils::fs::ExclusiveFileLock>,
}

impl<G: WriteGuard> NodeDatabase<G> {
    /// Composes the process adapter while retaining the same injected SQLite file and exclusive lease.
    pub fn process_journal(&self) -> Result<ProcessJournal<G>, Error> {
        let path = self.connection.path().ok_or(Error::InvalidSchema)?;
        let connection = Connection::open(path)?;
        connection.pragma_update(/*schema_name*/ None, "foreign_keys", "ON")?;
        connection.pragma_update(/*schema_name*/ None, "synchronous", "FULL")?;
        Ok(ProcessJournal {
            connection,
            guard: Arc::clone(&self.guard),
            _lease: Arc::clone(&self._lease),
        })
    }
}

impl<G: WriteGuard> ProcessJournal<G> {
    /// Registers a clone only after owned directory evidence was durably recorded.
    pub fn manage_clone(&self, record: &crate::CloneExecution) -> Result<(), Error> {
        let (input, target, progress): (String, String, String) = self.connection.query_row(
            "SELECT input,target,progress FROM clone_executions WHERE execution=?1",
            [record.command.execution_id.as_str()],
            |r| Ok((r.get(/*idx*/ 0)?, r.get(/*idx*/ 1)?, r.get(/*idx*/ 2)?)),
        )?;
        if serde_json::from_str::<ora_node_protocol::CloneRepositoryMessage>(&input)?
            != record.command
            || serde_json::from_str::<crate::CloneTarget>(&target)? != record.target
            || serde_json::from_str::<crate::CloneProgress>(&progress)? != record.progress
        {
            return Err(Error::IdentityConflict);
        }
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM managed_executions WHERE execution=?1)",
            [record.command.execution_id.as_str()],
            |r| r.get(/*idx*/ 0),
        )?;
        if exists {
            return Ok(());
        }
        if !matches!(
            record.progress,
            crate::CloneProgress::Pending(
                crate::ClonePhase::DirectoryCreated { .. } | crate::ClonePhase::Fetching { .. }
            ) | crate::CloneProgress::Unknown(
                crate::ClonePhase::DirectoryCreated { .. } | crate::ClonePhase::Fetching { .. }
            )
        ) {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Process)?;
        self.connection.execute(
            "INSERT INTO managed_executions VALUES (?1)",
            [record.command.execution_id.as_str()],
        )?;
        Ok(())
    }

    /// Persists how an attempt ended, once its stop evidence held, without copying process output
    /// or credentials.
    ///
    /// An attempt has exactly one ending: repeating the same one is accepted, a different one is
    /// a conflict, whichever of the three was recorded first.
    pub fn record_ending(&self, attempt: &AttemptId, ending: AttemptEnding) -> Result<(), Error> {
        match self.ending(attempt)? {
            Some(recorded) if recorded == ending => return Ok(()),
            Some(_) => return Err(Error::IdentityConflict),
            None => {}
        }
        self.guard.before_write(WritePoint::Process)?;
        match ending {
            AttemptEnding::Exited(code) => self.connection.execute(
                "INSERT INTO process_outcomes VALUES (?1,?2)",
                params![attempt.as_str(), code],
            )?,
            AttemptEnding::Terminated(signal) => self.connection.execute(
                "INSERT INTO process_terminations VALUES (?1,?2)",
                params![attempt.as_str(), signal],
            )?,
            AttemptEnding::Stopped => self
                .connection
                .execute("INSERT INTO process_stops VALUES (?1)", [attempt.as_str()])?,
        };
        Ok(())
    }

    /// Reads the ending an earlier pass or process recorded, so a settled attempt is never
    /// probed again after its lock file was removed.
    pub fn ending(&self, attempt: &AttemptId) -> Result<Option<AttemptEnding>, Error> {
        let code: Option<i32> = self
            .connection
            .query_row(
                "SELECT exit_code FROM process_outcomes WHERE run=?1",
                [attempt.as_str()],
                |r| r.get(/*idx*/ 0),
            )
            .optional()?;
        if let Some(code) = code {
            return Ok(Some(AttemptEnding::Exited(code)));
        }
        let signal: Option<i32> = self
            .connection
            .query_row(
                "SELECT signal FROM process_terminations WHERE run=?1",
                [attempt.as_str()],
                |r| r.get(/*idx*/ 0),
            )
            .optional()?;
        if let Some(signal) = signal {
            return Ok(Some(AttemptEnding::Terminated(signal)));
        }
        let stopped: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM process_stops WHERE run=?1)",
            [attempt.as_str()],
            |r| r.get(/*idx*/ 0),
        )?;
        Ok(stopped.then_some(AttemptEnding::Stopped))
    }

    /// Retains original attempts even after cleanup so recovery can still query their terminal evidence.
    pub fn attempts(&self, execution: &ExecutionId) -> Result<Vec<ProcessAttempt>, Error> {
        self.identities(
            "SELECT run FROM process_attempts WHERE execution=?1 ORDER BY rowid",
            execution,
        )
    }

    /// Lists every attempt of any kind whose stop evidence is still owed. A lock file that
    /// belongs to none of them was left by a command the database never recorded.
    pub fn unsettled(&self) -> Result<Vec<AttemptId>, Error> {
        self.connection
            .prepare(
                "SELECT run FROM process_attempts WHERE cleaned=0 UNION SELECT run FROM restore_process_attempts WHERE cleaned=0",
            )?
            .query_map([], |row| row.get::<_, String>(/*idx*/ 0))?
            .map(|id| AttemptId::parse(&id?).ok_or(Error::InvalidSchema))
            .collect()
    }

    /// Reads attempt identities only: rows written before Git ran directly hold a host intent in
    /// `data` that nothing decodes any more.
    fn identities(
        &self,
        query: &str,
        execution: &ExecutionId,
    ) -> Result<Vec<ProcessAttempt>, Error> {
        self.connection
            .prepare(query)?
            .query_map([execution.as_str()], |row| {
                row.get::<_, String>(/*idx*/ 0)
            })?
            .map(|id| {
                Ok(ProcessAttempt {
                    execution: execution.clone(),
                    id: AttemptId::parse(&id?).ok_or(Error::InvalidSchema)?,
                })
            })
            .collect()
    }

    /// Lists unfinished associations so normal shutdown cannot overlook a prior ambiguous exchange.
    pub fn pending_executions(&self) -> Result<Vec<ExecutionId>, Error> {
        let mut query = self
            .connection
            .prepare("SELECT DISTINCT execution FROM process_attempts WHERE cleaned=0")?;
        query
            .query_map([], |row| row.get::<_, String>(/*idx*/ 0))?
            .map(|id| Ok(ExecutionId::new(id?)))
            .collect()
    }
    /// Only an unstarted acceptance can enter managed execution without prior process evidence.
    /// Legacy in-flight direct Git remains blocked rather than being declared stopped by migration.
    pub fn manage(&self, execution: &Execution) -> Result<(), Error> {
        let id = execution.command.execution_id().as_str();
        let (input, progress): (String, String) = self.connection.query_row(
            "SELECT input,progress FROM executions WHERE execution=?1",
            [id],
            |row| Ok((row.get(/*idx*/ 0)?, row.get(/*idx*/ 1)?)),
        )?;
        if serde_json::from_str::<crate::Command>(&input)? != execution.command {
            return Err(Error::IdentityConflict);
        }
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM managed_executions WHERE execution=?1)",
            [id],
            |row| row.get(/*idx*/ 0),
        )?;
        if exists {
            return Ok(());
        }
        if execution.progress != Progress::Accepted
            || serde_json::from_str::<Progress>(&progress)? != Progress::Accepted
        {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Process)?;
        self.connection
            .execute("INSERT INTO managed_executions VALUES (?1)", [id])?;
        Ok(())
    }

    /// Records a unique attempt before its command is dispatched; a retry never reuses the ID.
    pub fn record(&self, attempt: &ProcessAttempt, command: &AttemptCommand) -> Result<(), Error> {
        let unfinished: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM executions WHERE execution=?1 AND state!='completed' UNION ALL SELECT 1 FROM clone_executions WHERE execution=?1 AND state!='completed' AND json_extract(progress, '$.evidence.phase')='dispatched')",
            [attempt.execution.as_str()],
            |row| row.get(/*idx*/ 0),
        )?;
        if !unfinished {
            return Err(Error::InvalidTransition);
        }
        // Snapshot clone has one durable attempt per workload step. A repeated attempt for the
        // same step is forbidden; the next step may start only after every earlier one is cleaned.
        let invalid_clone: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM clone_executions c WHERE c.execution=?1 AND (COALESCE(json_extract(c.progress, '$.evidence.step'),0) != (SELECT count(*) FROM process_attempts p WHERE p.execution=c.execution) OR EXISTS(SELECT 1 FROM process_attempts p WHERE p.execution=c.execution AND p.cleaned=0)))",
            [attempt.execution.as_str()], |r| r.get(/*idx*/ 0),
        )?;
        if invalid_clone {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Process)?;
        self.connection.execute(
            "INSERT INTO process_attempts VALUES (?1,?2,?3,0)",
            params![
                attempt.id.as_str(),
                attempt.execution.as_str(),
                serde_json::to_vec(command)?,
            ],
        )?;
        Ok(())
    }

    /// Retains every uncertain attempt until its stop evidence held.
    pub fn pending(&self, execution: &ExecutionId) -> Result<Vec<ProcessAttempt>, Error> {
        self.identities(
            "SELECT run FROM process_attempts WHERE execution=?1 AND cleaned=0 ORDER BY rowid",
            execution,
        )
    }

    /// Acknowledges stop evidence locally without deleting historical execution/attempt associations.
    pub fn cleaned(&self, attempt: &AttemptId) -> Result<(), Error> {
        self.guard.before_write(WritePoint::Process)?;
        self.connection.execute(
            "UPDATE process_attempts SET cleaned=1 WHERE run=?1",
            [attempt.as_str()],
        )?;
        Ok(())
    }
}
