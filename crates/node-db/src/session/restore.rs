//! Restore safety facts share the session lease and survive actor and Node restarts.
use super::*;
use crate::{AttemptCommand, AttemptId, ProcessAttempt};

/// Durable admission facts, rather than guesses from terminal reasons or missing process rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestorePhase {
    Pending,
    GitAdmitted,
    Restored,
    AgentAdmitted,
    LegacyUnknown,
}

/// Original attempt identity and a stop clock that is anchored exactly once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreAttempt {
    pub process: ProcessAttempt,
    pub guarantee: StopContainment,
    pub first_stop_at_ms: Option<i64>,
    pub stop_bound_at_ms: Option<i64>,
    pub stop_unconfirmed: bool,
}

impl<G: WriteGuard> SessionJournal<G> {
    /// Reads safety facts for delivery and recovery through the same narrow actor connection.
    pub fn restore_phase(&self, execution: &ExecutionId) -> Result<Option<RestorePhase>, Error> {
        phase(&self.lock()?.connection, execution)
    }

    /// Records completed restore before any plugin can be admitted.
    pub fn restored(&self, execution: &ExecutionId) -> Result<(), Error> {
        self.set_restore_phase(execution, "restored")
    }

    /// Fences plugin admission durably; a crash cannot make it look like an unstarted restore.
    pub fn agent_admitted(&self, execution: &ExecutionId) -> Result<(), Error> {
        self.set_restore_phase(execution, "agent_admitted")
    }

    /// Transitions never erase an incomplete or legacy-unknown restore.
    fn set_restore_phase(&self, execution: &ExecutionId, next: &str) -> Result<(), Error> {
        let db = self.lock()?;
        if !pending(&db.connection, execution)?.is_empty() {
            return Err(Error::ResourceConflict);
        }
        let current = phase(&db.connection, execution)?;
        if current.is_none() {
            return Ok(()); // A session without prior input needs no restore fact.
        }
        if !matches!(
            (current, next),
            (Some(RestorePhase::GitAdmitted), "restored")
                | (Some(RestorePhase::Restored), "agent_admitted")
        ) {
            return Err(Error::InvalidTransition);
        }
        db.guard.before_write(WritePoint::Progress)?;
        db.connection.execute(
            "UPDATE session_restores SET phase=?2 WHERE execution=?1",
            params![execution.as_str(), next],
        )?;
        Ok(())
    }

    /// Persists the attempt before its command is dispatched and can cause a write.
    pub fn record_restore_attempt(
        &self,
        attempt: &ProcessAttempt,
        command: &AttemptCommand,
        guarantee: StopContainment,
    ) -> Result<(), Error> {
        let mut db = self.lock()?;
        if !matches!(
            phase(&db.connection, &attempt.execution)?,
            Some(RestorePhase::Pending | RestorePhase::GitAdmitted)
        ) || !pending(&db.connection, &attempt.execution)?.is_empty()
        {
            return Err(Error::InvalidTransition);
        }
        let stopping: bool = db.connection.query_row(
            "SELECT first_stop_at_ms IS NOT NULL FROM session_restores WHERE execution=?1",
            [attempt.execution.as_str()],
            |row| row.get(/*idx*/ 0),
        )?;
        if stopping {
            return Err(Error::InvalidTransition);
        }
        let current = read(&db.connection, &attempt.execution)?.ok_or(Error::IdentityConflict)?;
        if current.state != ExecutionState::Running {
            return Err(Error::InvalidTransition);
        }
        db.guard.before_write(WritePoint::Process)?;
        let data = serde_json::to_vec(command)?;
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO restore_process_attempts(run,execution,data,guarantee) VALUES(?1,?2,?3,?4)", params![attempt.id.as_str(), attempt.execution.as_str(), data, match guarantee { StopContainment::BestEffort => "best_effort", StopContainment::Strong => "strong" }])?;
        tx.execute(
            "UPDATE session_restores SET phase='git_admitted' WHERE execution=?1",
            [attempt.execution.as_str()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The durable stop marker fences local staging as well as command admission.
    pub fn restore_stop_requested(&self, execution: &ExecutionId) -> Result<bool, Error> {
        let db = self.lock()?;
        Ok(db
            .connection
            .query_row(
                "SELECT first_stop_at_ms IS NOT NULL FROM session_restores WHERE execution=?1",
                [execution.as_str()],
                |row| row.get(/*idx*/ 0),
            )
            .optional()?
            .unwrap_or(/*default*/ false))
    }

    /// User cancellation and Node stop anchor before any command-boundary close side effect.
    pub fn request_session_restore_stop(
        &self,
        execution: &ExecutionId,
        now_ms: i64,
        bound_ms: i64,
    ) -> Result<(), Error> {
        let mut db = self.lock()?;
        let expires = now_ms
            .checked_add(bound_ms)
            .filter(|at| *at > now_ms)
            .ok_or(Error::InvalidTransition)?;
        db.guard.before_write(WritePoint::Process)?;
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("UPDATE session_restores SET first_stop_at_ms=?2,stop_bound_at_ms=?3 WHERE execution=?1 AND phase IN ('pending','git_admitted') AND first_stop_at_ms IS NULL", params![execution.as_str(),now_ms,expires])?;
        tx.execute("UPDATE restore_process_attempts SET first_stop_at_ms=(SELECT first_stop_at_ms FROM session_restores WHERE execution=?1),stop_bound_at_ms=(SELECT stop_bound_at_ms FROM session_restores WHERE execution=?1) WHERE execution=?1 AND first_stop_at_ms IS NULL AND cleaned=0", [execution.as_str()])?;
        tx.commit()?;
        Ok(())
    }

    /// Anchors before the stop request; retries and changed configuration cannot extend an
    /// existing clock.
    pub fn request_restore_stop(
        &self,
        attempt: &AttemptId,
        now_ms: i64,
        bound_ms: i64,
    ) -> Result<(), Error> {
        let db = self.lock()?;
        let expires = now_ms
            .checked_add(bound_ms)
            .filter(|at| *at > now_ms)
            .ok_or(Error::InvalidTransition)?;
        db.guard.before_write(WritePoint::Process)?;
        db.connection.execute("UPDATE restore_process_attempts SET first_stop_at_ms=?2,stop_bound_at_ms=?3 WHERE run=?1 AND first_stop_at_ms IS NULL AND cleaned=0", params![attempt.as_str(), now_ms, expires])?;
        Ok(())
    }

    /// Leaves every identity and conflict responsibility intact when its independent bound expires.
    pub fn mark_restore_stop_unconfirmed(
        &self,
        attempt: &AttemptId,
        now_ms: i64,
    ) -> Result<(), Error> {
        let db = self.lock()?;
        db.guard.before_write(WritePoint::Process)?;
        db.connection.execute("UPDATE restore_process_attempts SET stop_unconfirmed=1 WHERE run=?1 AND cleaned=0 AND stop_bound_at_ms<=?2", params![attempt.as_str(), now_ms])?;
        Ok(())
    }

    /// The caller must hold the attempt's stop evidence: its lock could be taken again.
    pub fn confirm_restore_stop(&self, attempt: &AttemptId) -> Result<(), Error> {
        let db = self.lock()?;
        db.guard.before_write(WritePoint::Process)?;
        db.connection.execute(
            "UPDATE restore_process_attempts SET cleaned=1 WHERE run=?1",
            [attempt.as_str()],
        )?;
        Ok(())
    }

    /// Pending identities stay queryable even after the session stops polling them.
    pub fn pending_restore_attempts(
        &self,
        execution: &ExecutionId,
    ) -> Result<Vec<RestoreAttempt>, Error> {
        pending(&self.lock()?.connection, execution)
    }

    /// Keeps the desired terminal fact while stop evidence is missing; the first cause wins.
    pub fn defer_restore_end(
        &self,
        execution: &ExecutionId,
        ended: &AgentSessionEnded,
    ) -> Result<(), Error> {
        let db = self.lock()?;
        db.guard.before_write(WritePoint::Progress)?;
        db.connection.execute("UPDATE session_restores SET deferred_end=?2 WHERE execution=?1 AND deferred_end IS NULL", params![execution.as_str(), serde_json::to_string(ended)?])?;
        Ok(())
    }

    /// A late proof settles the original execution exactly once through the normal terminal writer.
    pub fn deferred_restore_end(
        &self,
        execution: &ExecutionId,
    ) -> Result<Option<AgentSessionEnded>, Error> {
        let db = self.lock()?;
        let data: Option<String> = db
            .connection
            .query_row(
                "SELECT deferred_end FROM session_restores WHERE execution=?1",
                [execution.as_str()],
                |r| r.get(/*idx*/ 0),
            )
            .optional()?
            .flatten();
        data.map(|data| serde_json::from_str(&data).map_err(Error::from))
            .transpose()
    }
}

/// Reads only ledger facts; this must be checked before HEAD or any checkout mutation.
pub(super) fn phase(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
) -> Result<Option<RestorePhase>, Error> {
    let value: Option<String> = connection
        .query_row(
            "SELECT phase FROM session_restores WHERE execution=?1",
            [execution.as_str()],
            |r| r.get(/*idx*/ 0),
        )
        .optional()?;
    value
        .map(|value| match value.as_str() {
            "pending" => Ok(RestorePhase::Pending),
            "git_admitted" => Ok(RestorePhase::GitAdmitted),
            "restored" => Ok(RestorePhase::Restored),
            "agent_admitted" => Ok(RestorePhase::AgentAdmitted),
            "legacy_unknown" => Ok(RestorePhase::LegacyUnknown),
            _ => Err(Error::InvalidSchema),
        })
        .transpose()
}

/// Shares the same pending predicate between the terminal barrier, admission and delivery.
pub(super) fn pending(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
) -> Result<Vec<RestoreAttempt>, Error> {
    connection.prepare("SELECT run,first_stop_at_ms,stop_bound_at_ms,stop_unconfirmed,guarantee FROM restore_process_attempts WHERE execution=?1 AND cleaned=0 ORDER BY rowid")?
        .query_map([execution.as_str()], |r| Ok((r.get::<_, String>(/*idx*/ 0)?,r.get(/*idx*/ 1)?,r.get(/*idx*/ 2)?,r.get(/*idx*/ 3)?,r.get::<_,String>(/*idx*/ 4)?)))?
        .map(|row| {
            let (id,first_stop_at_ms,stop_bound_at_ms,stop_unconfirmed, guarantee) = row?;
            let guarantee = match guarantee.as_str() { "best_effort" => StopContainment::BestEffort, "strong" => StopContainment::Strong, _ => return Err(Error::InvalidSchema) };
            Ok(RestoreAttempt { guarantee, process: ProcessAttempt { execution: execution.clone(), id: AttemptId::parse(&id).ok_or(Error::InvalidSchema)? }, first_stop_at_ms, stop_bound_at_ms, stop_unconfirmed })
        }).collect()
}
