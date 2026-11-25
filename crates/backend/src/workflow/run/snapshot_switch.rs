//! Loads, validates, and applies a published-snapshot switch before a failed run resumes.

use crate::error::BackendError;
use ora_application::{
    ApplicationError, SkillMaterializationReceipt, SnapshotIncompatibility, SnapshotSwitchPlan,
    WorkflowGraph, WorkflowRepository, WorkflowRunEngineRepository, WorkflowRunPayload,
    WorkflowRunRepository, WorkflowVariablePool, plan_snapshot_switch,
};
use ora_contracts::WorkflowRunLocale;
use ora_db::{
    RepositoryPool, SqliteWorkflowRepository, SqliteWorkflowRunEngineRepository,
    SqliteWorkflowRunRepository,
};
use ora_domain::{Workflow, WorkflowRun, WorkflowRunId, WorkflowSnapshot, WorkflowSnapshotId};
use ora_logging::ora_info;

const DRAFT_VERSION: &str = "draft";

/// Run, workflow, and snapshot rows needed to decide whether a resume can switch versions.
pub(super) struct SnapshotSwitchContext {
    pub run: WorkflowRun,
    pub workflow: Workflow,
    pub current: WorkflowSnapshot,
    pub published: Option<WorkflowSnapshot>,
}

/// Reads the run, its workflow, the snapshot the run currently pins, and the published snapshot.
pub(super) fn load_context(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
) -> Result<SnapshotSwitchContext, BackendError> {
    let run = SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(run_id)
        .map_err(|error| BackendError::internal("failed to load workflow run", error))?
        .ok_or_else(|| {
            BackendError::from(ApplicationError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            })
        })?;
    let workflow_repo = SqliteWorkflowRepository::new(pool.clone());
    let workflow = workflow_repo
        .find_workflow(&run.workflow_id)
        .map_err(|error| BackendError::internal("failed to load workflow for resume", error))?
        .ok_or_else(|| {
            BackendError::from(ApplicationError::WorkflowNotFound {
                workflow_id: run.workflow_id.to_string(),
            })
        })?;
    let current = workflow_repo
        .find_snapshot_by_id(&run.workflow_id, &run.snapshot_id)
        .map_err(|error| BackendError::internal("failed to load run snapshot", error))?
        .ok_or_else(|| {
            BackendError::from(ApplicationError::WorkflowSnapshotNotFoundById {
                snapshot_id: run.snapshot_id.to_string(),
            })
        })?;
    let published = match workflow.published_snapshot_id.as_ref() {
        Some(published_id) => workflow_repo
            .find_snapshot_by_id(&run.workflow_id, published_id)
            .map_err(|error| {
                BackendError::internal("failed to load published workflow snapshot", error)
            })?,
        None => None,
    };
    Ok(SnapshotSwitchContext {
        run,
        workflow,
        current,
        published,
    })
}

/// Parses both graphs, lists live node runs, and dry-runs the snapshot-switch plan.
pub(super) fn check_switch(
    pool: &RepositoryPool,
    context: &SnapshotSwitchContext,
    target: &WorkflowSnapshot,
) -> Result<SnapshotSwitchPlan, BackendError> {
    let old_graph = WorkflowGraph::parse(&context.current.graph).map_err(ApplicationError::from)?;
    let new_graph = WorkflowGraph::parse(&target.graph).map_err(ApplicationError::from)?;
    let node_runs = SqliteWorkflowRunEngineRepository::new(pool.clone())
        .list_node_runs(&context.run.id)
        .map_err(|error| {
            BackendError::internal("failed to list node runs for snapshot switch", error)
        })?;
    let variable_pool = payload_pool(context.run.payload.as_deref());
    plan_snapshot_switch(&old_graph, &new_graph, &node_runs, &variable_pool).map_err(
        |SnapshotIncompatibility { reason }| {
            BackendError::from(ApplicationError::WorkflowSnapshotIncompatibleWithResume { reason })
        },
    )
}

/// The exact validated snapshot and receipt that may be committed after rollback succeeds.
pub(super) struct PreparedSwitch {
    context: SnapshotSwitchContext,
    target: WorkflowSnapshot,
    payload_json: String,
}

impl PreparedSwitch {
    /// Commits the prevalidated payload without re-reading plugins after the worktree changes.
    pub(super) fn apply(self, pool: &RepositoryPool, now: i64) -> Result<(), BackendError> {
        let switched = SqliteWorkflowRunEngineRepository::new(pool.clone())
            .switch_run_snapshot(
                &self.context.run.id,
                &self.target.id,
                &self.payload_json,
                now,
            )
            .map_err(|error| {
                BackendError::internal("failed to switch workflow run snapshot", error)
            })?;
        if !switched {
            return Err(BackendError::from(
                ApplicationError::WorkflowRunNotResumable,
            ));
        }
        ora_info!(
            run_id = %self.context.run.id,
            old_snapshot_id = %self.context.current.id,
            old_version = %self.context.current.version,
            new_snapshot_id = %self.target.id,
            new_version = %self.target.version,
            "switched workflow run snapshot for resume"
        );
        Ok(())
    }
}

/// Validates the graph the resume will dispatch and prepares a switch only when its pin changes.
pub(super) fn prepare_resume(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
    snapshot_id: Option<&str>,
    resolve: impl FnOnce(
        &WorkflowGraph,
        Option<&SkillMaterializationReceipt>,
    ) -> Result<SkillMaterializationReceipt, BackendError>,
) -> Result<Option<PreparedSwitch>, BackendError> {
    let context = load_context(pool, run_id)?;
    let Some(snapshot_id) = snapshot_id.filter(|id| *id != context.run.snapshot_id.as_ref()) else {
        let graph = WorkflowGraph::parse(&context.current.graph).map_err(ApplicationError::from)?;
        let frozen = match context.run.payload.as_deref() {
            Some(raw) => Some(
                serde_json::from_str::<WorkflowRunPayload>(raw)
                    .map_err(|error| ApplicationError::WorkflowRunStartFailed {
                        message: format!("failed to parse frozen workflow run payload: {error}"),
                    })?
                    .skill_materialization,
            ),
            None => None,
        };
        resolve(&graph, frozen.as_ref())?;
        return Ok(None);
    };
    let target = SqliteWorkflowRepository::new(pool.clone())
        .find_snapshot_by_id(&context.workflow.id, &WorkflowSnapshotId::new(snapshot_id))
        .map_err(|error| BackendError::internal("failed to load target snapshot", error))?
        .ok_or_else(|| {
            BackendError::from(ApplicationError::WorkflowSnapshotNotFoundById {
                snapshot_id: snapshot_id.to_string(),
            })
        })?;
    if target.version == DRAFT_VERSION {
        return Err(incompatible("draft_snapshot"));
    }
    if target.workflow_id != context.workflow.id {
        return Err(incompatible("snapshot_not_in_workflow"));
    }
    let plan = check_switch(pool, &context, &target)?;
    let new_graph = WorkflowGraph::parse(&target.graph).map_err(ApplicationError::from)?;
    let receipt = resolve(&new_graph, /*frozen_receipt*/ None)?;
    let mut payload = parsed_payload(context.run.payload.as_deref());
    payload.skill_materialization = receipt;
    payload.variable_pool = plan.variable_pool;
    payload.start_node_id = new_graph.start_node().map(|node| node.id.clone());
    let payload_json = serde_json::to_string(&payload).map_err(|error| {
        ApplicationError::WorkflowRunStartFailed {
            message: format!("failed to serialize switched workflow run payload: {error}"),
        }
    })?;
    Ok(Some(PreparedSwitch {
        context,
        target,
        payload_json,
    }))
}

fn incompatible(reason: &str) -> BackendError {
    BackendError::from(ApplicationError::WorkflowSnapshotIncompatibleWithResume {
        reason: reason.to_string(),
    })
}

fn payload_pool(payload: Option<&str>) -> WorkflowVariablePool {
    parsed_payload(payload).variable_pool
}

fn parsed_payload(payload: Option<&str>) -> WorkflowRunPayload {
    payload
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| WorkflowRunPayload::new(WorkflowRunLocale::ZhCn, Default::default()))
}
