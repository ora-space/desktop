//! Durable run-local edit receipts. The executor owns stopping, dispatch and per-kind planning.
use crate::{DatabaseError, RepositoryPool};
use ora_application::{WorkflowGraph, WorkflowVariablePool};
use ora_contracts::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

mod attempts;
mod configuration;
mod operations;
pub use operations::DebugClaim;
#[cfg(test)]
mod tests;

/// Domain outcomes that callers can distinguish from storage failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DebugRejection {
    #[error("missing_input_evidence")]
    MissingInputEvidence,
    #[error("stale_preview")]
    StalePreview,
    #[error("request_conflict")]
    RequestConflict,
    #[error("operation_in_progress")]
    OperationInProgress,
    #[error("invalid_target")]
    InvalidTarget,
    #[error("invalid_patch at {path}: {reason}")]
    InvalidPatch { path: String, reason: String },
}

/// The immutable attempt inputs and candidate graph supplied to the server's node strategy.
pub struct DebugPlanningInput {
    pub target: WorkflowDebugTarget,
    pub candidate: WorkflowGraph,
    pub pool: WorkflowVariablePool,
    pub entry: Value,
    pub dispatch: Option<Value>,
    pub current_attempts: Vec<ora_domain::WorkflowNodeRun>,
    pub current_scopes: Vec<ora_domain::WorkflowExecutionScope>,
    pub current_payload: Option<ora_application::WorkflowRunPayload>,
    pub predicted_branch: Option<String>,
}

/// The SQLite owner for debug history, input evidence and confirmation admission.
#[derive(Clone)]
pub struct WorkflowDebugStore {
    pool: RepositoryPool,
}

impl WorkflowDebugStore {
    /// Uses the same database pool as workflow scheduling.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Creates a receipt only for a target and patch accepted by the server-side planning strategy.
    /// The callback performs pure planning with facts from the same read transaction.
    pub fn preview(
        &self,
        request: &PreviewWorkflowDebugRequest,
        plan: impl FnOnce(&DebugPlanningInput) -> Result<WorkflowDebugImpact, DebugRejection>,
    ) -> Result<WorkflowDebugPreview, DatabaseError> {
        self.pool.with_connection_mut(|connection| {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if operations::blocked(&tx, &request.run_id)? {
                return Err(DebugRejection::OperationInProgress.into());
            }
            let (revision, epoch) = run_version(&tx, &request.run_id)?;
            if revision != request.base_revision {
                return Err(DebugRejection::StalePreview.into());
            }
            let input = configuration::prepare(&tx, request)?;
            let impact = plan(&input)?;
            let preview = WorkflowDebugPreview {
                preview_id: Uuid::new_v4().to_string(),
                base_revision: revision,
                target: input.target,
                impact,
                workspace: WorkflowDebugWorkspace::Keep,
                external_actions_may_repeat: true,
            };
            tx.execute(
                "INSERT INTO workflow_debug_previews(id,run_id,target,base_revision,patch,plan,epoch)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![preview.preview_id,request.run_id,request.node_run_id,revision,
                    serde_json::to_string(&request.patch)?,serde_json::to_string(&preview)?,epoch],
            )?;
            tx.commit()?;
            Ok(preview)
        })
    }

    /// Saves the exact values passed to template rendering and the handoff assembled by the executor.
    /// A cancelled or duplicate driver cannot replace another dispatch's evidence.
    pub fn record_agent_dispatch(
        &self,
        run_id: &str,
        node_run_id: &str,
        dispatch: &Value,
    ) -> Result<bool, DatabaseError> {
        self.pool.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE workflow_debug_attempts SET dispatch_input=?3 WHERE node_run_id=?2
                    AND dispatch_input IS NULL AND EXISTS (
                        SELECT 1 FROM workflow_node_runs n WHERE n.id=?2 AND n.run_id=?1
                        AND n.is_deleted=0 AND n.status=1)",
                params![run_id, node_run_id, serde_json::to_string(dispatch)?],
            )?;
            Ok(changed == 1)
        })
    }
}

/// Reads the run-wide execution epoch under the same transaction as the preview or claim.
fn run_version(db: &Connection, run: &str) -> Result<(u32, i64), DatabaseError> {
    db.query_row(
        "SELECT COALESCE((SELECT MAX(revision) FROM workflow_debug_operations WHERE run_id=r.id),0),
           r.debug_epoch FROM workflow_runs r WHERE r.id=?1 AND r.is_deleted=0 AND r.run_status<>0",
        [run], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?.ok_or(DebugRejection::InvalidTarget.into())
}
