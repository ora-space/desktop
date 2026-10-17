//! Actor queries resolve checkout paths exclusively from successful clone evidence.
use super::*;
use std::path::PathBuf;

impl<G: WriteGuard> SessionJournal<G> {
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
