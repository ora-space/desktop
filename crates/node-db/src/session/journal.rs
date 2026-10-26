//! Actor queries resolve checkout paths exclusively from successful clone evidence.
use super::*;
use std::path::PathBuf;

impl<G: WriteGuard> SessionJournal<G> {
    /// Only the live actor records this after plugin and restore stop confirmation, before ending.
    pub fn confirm_writer_stop(&self, execution: &ExecutionId) -> Result<(), Error> {
        let db = self.lock()?;
        let session = read(&db.connection, execution)?.ok_or(Error::IdentityConflict)?;
        if session.state != ExecutionState::Running {
            return Err(Error::InvalidTransition);
        }
        db.guard.before_write(WritePoint::Progress)?;
        db.connection.execute(
            "INSERT INTO session_writer_stops(execution) VALUES(?1) ON CONFLICT DO NOTHING",
            [execution.as_str()],
        )?;
        Ok(())
    }

    /// Neither a restart-generated terminal nor elapsed time invents process-stop evidence.
    pub fn writer_stop_confirmed(&self, execution: &ExecutionId) -> Result<bool, Error> {
        self.lock()?
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM session_writer_stops WHERE execution=?1)",
                [execution.as_str()],
                |row| row.get(/*idx*/ 0),
            )
            .map_err(Error::from)
    }

    /// Records possible provider-side history before issuing session creation, including crashes.
    pub fn admit_history(&self, execution: &ExecutionId) -> Result<(), Error> {
        let db = self.lock()?;
        let session = read(&db.connection, execution)?.ok_or(Error::IdentityConflict)?;
        if session.state != ExecutionState::Running {
            return Err(Error::InvalidTransition);
        }
        db.guard.before_write(WritePoint::Progress)?;
        db.connection.execute(
            "INSERT INTO session_history_admissions(execution) VALUES(?1) ON CONFLICT DO NOTHING",
            [execution.as_str()],
        )?;
        Ok(())
    }

    /// Only a new, never-admitted execution can prove that no provider history ever existed.
    pub fn history_admitted(&self, execution: &ExecutionId) -> Result<bool, Error> {
        self.lock()?
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM session_history_admissions WHERE execution=?1)",
                [execution.as_str()],
                |row| row.get(/*idx*/ 0),
            )
            .map_err(Error::from)
    }
    /// A released actor with unknown stop evidence is not a sealed terminal session.
    pub fn session_completed(&self, execution: &ExecutionId) -> Result<bool, Error> {
        Ok(read(&self.lock()?.connection, execution)?
            .is_some_and(|record| matches!(record.state, ExecutionState::Completed(_))))
    }

    /// Returns the exact persisted destination only for a completed successful clone. Missing,
    /// pending, failed and other execution families do not supply a checkout.
    pub fn checkout(&self, execution: &ExecutionId) -> Result<Option<PathBuf>, Error> {
        let db = self.lock()?;
        let row: Option<(String, String)> = db
            .connection
            .query_row(
                "SELECT target,progress FROM clone_executions WHERE execution=?1",
                [execution.as_str()],
                |r| Ok((r.get(/*idx*/ 0)?, r.get(/*idx*/ 1)?)),
            )
            .optional()?;
        let Some((target, progress)) = row else {
            return Ok(None);
        };
        let progress: CloneProgress = serde_json::from_str(&progress)?;
        match progress {
            CloneProgress::Completed(result) => match *result {
                CloneExecutionResult::CloneReady(_) => {
                    Ok(Some(serde_json::from_str::<CloneTarget>(&target)?.path))
                }
                CloneExecutionResult::CloneFailed(_) => Ok(None),
            },
            CloneProgress::Pending(_) | CloneProgress::Unknown(_) => Ok(None),
        }
    }
}
