//! The run-wide fence held by a confirmation that is stopping, or failed without a released stop.
use super::*;

/// Reads the partial unique index's live-fence condition.
pub(crate) fn held(db: &Connection, run: &str) -> Result<bool, rusqlite::Error> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM workflow_debug_operations WHERE run_id=?1
        AND (state='stopping' OR (state='stop_failed' AND released=0)))",
        [run],
        |r| r.get(0),
    )
}

/// Rejects new work while an earlier confirmation still holds the fence.
///
/// Runs inside the caller's write transaction, so the check and the write it guards cannot be
/// separated by a concurrent confirmation.
pub(crate) fn ensure_inactive(db: &Connection, run: &str) -> Result<(), DatabaseError> {
    match held(db, run)? {
        true => Err(DebugRejection::OperationActive.into()),
        false => Ok(()),
    }
}

impl WorkflowDebugStore {
    /// Refuses new work on a run whose debug confirmation is stopping or failed with the fence
    /// still held.
    ///
    /// Entry points outside this crate call it before writing or dispatching for the run. Reading
    /// an accepted request back by its key stays available through [`Self::claim`].
    pub fn ensure_debug_operation_inactive(&self, run: &str) -> Result<(), DatabaseError> {
        self.pool.with_connection(|db| ensure_inactive(db, run))
    }
}
