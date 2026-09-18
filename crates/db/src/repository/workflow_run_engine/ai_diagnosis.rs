use ora_application::RepositoryError;
use ora_domain::WorkflowNodeRunId;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::engine_repository_error_from_database;
use super::payload_json::merge_payload_keys;
use crate::repository::RepositoryPool;

/// Merges `payload.ai_diagnosis` onto one live node-run row, overwriting a previous guess.
///
/// A missing row is a no-op so a diagnosis that finishes after resume cannot fail the request.
pub(super) fn record_node_ai_diagnosis(
    pool: &RepositoryPool,
    node_run_id: &WorkflowNodeRunId,
    diagnosis_json: &str,
    now: i64,
) -> Result<(), RepositoryError> {
    pool.with_connection_mut(|connection| {
        let diagnosis: serde_json::Value = serde_json::from_str(diagnosis_json)?;
        let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT payload FROM workflow_node_runs WHERE id = ?1 AND is_deleted = 0",
                params![node_run_id.as_ref()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        let Some(existing) = existing else {
            return Ok(());
        };
        let payload = merge_payload_keys(existing.as_deref(), [("ai_diagnosis", diagnosis)])?;
        transaction.execute(
            "UPDATE workflow_node_runs SET payload = ?2, updated_at = ?3
             WHERE id = ?1 AND is_deleted = 0",
            params![node_run_id.as_ref(), payload, now],
        )?;
        transaction.commit()?;
        Ok(())
    })
    .map_err(engine_repository_error_from_database)
}
