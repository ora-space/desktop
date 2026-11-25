//! Durable run-local edit receipts. The executor owns stopping, dispatch and per-kind planning.
use crate::{DatabaseError, RepositoryPool};
use ora_application::{WorkflowGraph, WorkflowVariablePool};
use ora_contracts::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

/// The settle transaction handed to `apply_after_stop`'s projection closure; re-exported so
/// coordinators outside this crate can name the parameter without depending on rusqlite directly.
pub use rusqlite::Transaction;

mod attempts;
mod configuration;
pub(crate) mod fence;
mod normalize;
mod operations;
mod pending;
mod read_set;
mod recovery;
pub use operations::DebugClaim;
#[cfg(test)]
mod tests;

/// Domain outcomes that callers can distinguish from storage failures.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum DebugRejection {
    #[error("missing_input_evidence")]
    MissingInputEvidence,
    #[error("stale_preview")]
    StalePreview,
    #[error("request_conflict")]
    RequestConflict,
    #[error("debug_operation_active")]
    OperationActive,
    #[error("missing_input_evidence: {0}")]
    MissingFact(String),
    #[error("invalid_target")]
    InvalidTarget,
    #[error("invalid_patch at {path}: {reason}")]
    InvalidPatch { path: String, reason: String },
}

/// The immutable attempt inputs and candidate graph supplied to the server's node strategy.
pub struct DebugPlanningInput {
    read_nodes: std::collections::BTreeSet<String>,
    pub target: WorkflowDebugTarget,
    pub candidate: WorkflowGraph,
    pub pool: WorkflowVariablePool,
    pub entry: Value,
    pub dispatch: Option<Value>,
    pub current_attempts: Vec<ora_domain::WorkflowNodeRun>,
    pub current_scopes: Vec<ora_domain::WorkflowExecutionScope>,
    pub current_payload: Option<ora_application::WorkflowRunPayload>,
    pub predicted_branch: Option<String>,
    pub member_configurations: std::collections::BTreeMap<String, Value>,
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
            fence::ensure_inactive(&tx, &request.run_id)?;
            let revision = run_version(&tx, &request.run_id)?;
            if revision != request.base_revision {
                return Err(DebugRejection::StalePreview.into());
            }
            let input = configuration::prepare(&tx, request)?;
            let impact = plan(&input)?;
            let nodes = serde_json::to_string(&input.read_nodes)?;
            let facts = read_set::facts(&tx, &request.run_id, &nodes)?;
            let kind = input.candidate.execution_node(&input.target.node_id).ok_or(DebugRejection::InvalidTarget)?.node_type.as_str();
            let preview = WorkflowDebugPreview {
                preview_id: Uuid::new_v4().to_string(),
                base_revision: revision,
                target: input.target,
                member_configurations: (!input.member_configurations.is_empty()).then_some(input.member_configurations),
                impact,
                workspace: WorkflowDebugWorkspace::Keep,
                external_actions_may_repeat: true,
            };
            tx.execute(
                "INSERT INTO workflow_debug_previews(id,run_id,base_revision,patch,plan,snapshot_id,root_scope,node_type,read_nodes,read_facts)
                 SELECT ?1,r.id,?3,?4,?5,r.snapshot_id,root.scope_id,?6,?7,?8 FROM workflow_runs r
                 JOIN workflow_run_root_scopes root ON root.run_id=r.id WHERE r.id=?2",
                params![preview.preview_id,request.run_id,revision,
                    serde_json::to_string(&request.patch)?,serde_json::to_string(&preview)?,kind,nodes,serde_json::to_string(&facts)?],
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

/// Reads the active configuration revision under the same transaction as the preview or claim.
/// A snapshot switch raises it like an applied edit does, through a baseline revision.
fn run_version(db: &Connection, run: &str) -> Result<u32, DatabaseError> {
    db.query_row(
        "SELECT COALESCE((SELECT MAX(revision) FROM workflow_debug_revisions WHERE run_id=r.id),0) FROM workflow_runs r WHERE r.id=?1 AND r.is_deleted=0 AND r.run_status<>0",
        [run], |row| row.get(0),
    ).optional()?.ok_or(DebugRejection::InvalidTarget.into())
}
