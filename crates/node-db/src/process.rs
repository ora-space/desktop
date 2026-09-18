use crate::{Error, Execution, NodeDatabase, Progress, WriteGuard, WritePoint};
use ora_node_protocol::ExecutionId;
use ora_process_protocol::{HostRunIntent, RunId};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

/// Node-owned association persisted before a host can accept a Git side effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessAttempt {
    pub execution: ExecutionId,
    pub host_directory: PathBuf,
    pub expected_uid: u32,
    pub intent: HostRunIntent,
}

/// A narrow second connection shares the original database lease, not a second Node authority.
/// Its transactions finish before host exchanges or business-state transactions begin.
pub struct ProcessJournal<G: WriteGuard> {
    connection: Connection,
    guard: Arc<G>,
    _lease: Arc<crate::Lease>,
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
            crate::CloneProgress::Pending(crate::ClonePhase::DirectoryCreated { .. })
                | crate::CloneProgress::Unknown(crate::ClonePhase::DirectoryCreated { .. })
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

    /// Persists only an observed exit code, without copying process output or credentials.
    pub fn record_outcome(&self, run: RunId, exit_code: i32) -> Result<(), Error> {
        self.guard.before_write(WritePoint::Process)?;
        self.connection.execute(
            "INSERT INTO process_outcomes VALUES (?1,?2) ON CONFLICT(run) DO NOTHING",
            params![run.to_string(), exit_code],
        )?;
        let saved: i32 = self.connection.query_row(
            "SELECT exit_code FROM process_outcomes WHERE run=?1",
            [run.to_string()],
            |r| r.get(/*idx*/ 0),
        )?;
        if saved != exit_code {
            return Err(Error::IdentityConflict);
        }
        Ok(())
    }

    /// Retains original attempts even after cleanup so recovery can still query their terminal evidence.
    pub fn attempts(&self, execution: &ExecutionId) -> Result<Vec<ProcessAttempt>, Error> {
        self.connection
            .prepare("SELECT data FROM process_attempts WHERE execution=?1 ORDER BY rowid")?
            .query_map([execution.as_str()], |r| {
                r.get::<_, Vec<u8>>(/*idx*/ 0)
            })?
            .map(|data| Ok(ora_process_protocol::decode_guardian_payload(&data?)?))
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

    /// Records a unique attempt before sending CreateScope or Start; retry never invents a new ID.
    pub fn record(&self, attempt: &ProcessAttempt) -> Result<(), Error> {
        let unfinished: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM executions WHERE execution=?1 AND state!='completed' UNION ALL SELECT 1 FROM clone_executions WHERE execution=?1 AND state!='completed' AND json_extract(progress, '$.evidence.phase')='dispatched')",
            [attempt.execution.as_str()],
            |row| row.get(/*idx*/ 0),
        )?;
        if !unfinished {
            return Err(Error::InvalidTransition);
        }
        let repeated_clone: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM clone_executions JOIN process_attempts USING(execution) WHERE execution=?1)",
            [attempt.execution.as_str()], |r| r.get(/*idx*/ 0),
        )?;
        if repeated_clone {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Process)?;
        let encoded = ora_process_protocol::encode_guardian_frame(attempt)?;
        self.connection.execute(
            "INSERT INTO process_attempts VALUES (?1,?2,?3,0)",
            params![
                attempt.intent.run.to_string(),
                attempt.execution.as_str(),
                &encoded[4..],
            ],
        )?;
        Ok(())
    }

    /// Retains every uncertain attempt until the original host/guardian has proved Scope closure.
    pub fn pending(&self, execution: &ExecutionId) -> Result<Vec<ProcessAttempt>, Error> {
        let mut query = self.connection.prepare(
            "SELECT data FROM process_attempts WHERE execution=?1 AND cleaned=0 ORDER BY rowid",
        )?;
        query
            .query_map([execution.as_str()], |row| {
                row.get::<_, Vec<u8>>(/*idx*/ 0)
            })?
            .map(|data| Ok(ora_process_protocol::decode_guardian_payload(&data?)?))
            .collect()
    }

    /// Acknowledges observed cleanup locally without deleting historical execution/Run associations.
    pub fn cleaned(&self, run: RunId) -> Result<(), Error> {
        self.guard.before_write(WritePoint::Process)?;
        self.connection.execute(
            "UPDATE process_attempts SET cleaned=1 WHERE run=?1",
            [run.to_string()],
        )?;
        Ok(())
    }
}
