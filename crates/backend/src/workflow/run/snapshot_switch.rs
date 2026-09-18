//! Loads, validates, and applies a published-snapshot switch before a failed run resumes.

use super::prerequisites::SkillRoleWorkspaceInitializer;
use crate::error::BackendError;
use ora_application::{
    ApplicationError, SnapshotIncompatibility, SnapshotSwitchPlan, StartPrerequisitesError,
    WorkflowGraph, WorkflowRepository, WorkflowRunEngineRepository, WorkflowRunPayload,
    WorkflowRunRepository, WorkflowRunWorkspaceInitializer, WorkflowVariablePool,
    plan_snapshot_switch,
};
use ora_contracts::WorkflowRunLocale;
use ora_db::{
    RepositoryPool, SqliteWorkflowRepository, SqliteWorkflowRunEngineRepository,
    SqliteWorkflowRunRepository,
};
use ora_domain::{Workflow, WorkflowRun, WorkflowRunId, WorkflowSnapshot, WorkflowSnapshotId};
use ora_logging::ora_info;
use std::path::Path;

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

/// Rebuilds skill materialization, persists the migrated payload, and retargets `snapshot_id`.
pub(super) fn apply_switch(
    pool: &RepositoryPool,
    skills_root: &Path,
    workspace_root: &Path,
    context: &SnapshotSwitchContext,
    target: &WorkflowSnapshot,
    plan: SnapshotSwitchPlan,
    now: i64,
) -> Result<(), BackendError> {
    if target.version == DRAFT_VERSION {
        return Err(incompatible("draft_snapshot"));
    }
    if target.workflow_id != context.workflow.id {
        return Err(incompatible("snapshot_not_in_workflow"));
    }
    let new_graph = WorkflowGraph::parse(&target.graph).map_err(ApplicationError::from)?;
    let initializer = SkillRoleWorkspaceInitializer::new(skills_root.to_path_buf(), pool.clone())
        .map_err(|error| ApplicationError::WorkflowRunStartFailed {
        message: error.to_string(),
    })?;
    let receipt = initializer
        .initialize_workspace(&new_graph, workspace_root)
        .map_err(start_prerequisites_error)?;
    let mut payload = parsed_payload(context.run.payload.as_deref());
    payload.skill_materialization = receipt;
    payload.variable_pool = plan.variable_pool;
    payload.start_node_id = new_graph.start_node().map(|node| node.id.clone());
    let payload_json = serde_json::to_string(&payload).map_err(|error| {
        ApplicationError::WorkflowRunStartFailed {
            message: format!("failed to serialize switched workflow run payload: {error}"),
        }
    })?;
    let switched = SqliteWorkflowRunEngineRepository::new(pool.clone())
        .switch_run_snapshot(&context.run.id, &target.id, &payload_json, now)
        .map_err(|error| BackendError::internal("failed to switch workflow run snapshot", error))?;
    if !switched {
        return Err(BackendError::from(
            ApplicationError::WorkflowRunNotResumable,
        ));
    }
    ora_info!(
        run_id = %context.run.id,
        old_snapshot_id = %context.current.id,
        old_version = %context.current.version,
        new_snapshot_id = %target.id,
        new_version = %target.version,
        "switched workflow run snapshot for resume"
    );
    Ok(())
}

/// Applies `target` when it differs from the run's current snapshot.
///
/// Returns `true` when a snapshot switch committed so callers can publish a run invalidation.
pub(super) fn switch_if_requested(
    pool: &RepositoryPool,
    skills_root: &Path,
    workspace_root: &Path,
    run_id: &WorkflowRunId,
    snapshot_id: Option<&str>,
    now: i64,
) -> Result<bool, BackendError> {
    let Some(snapshot_id) = snapshot_id else {
        return Ok(false);
    };
    let context = load_context(pool, run_id)?;
    if snapshot_id == context.run.snapshot_id.as_ref() {
        return Ok(false);
    }
    let target = SqliteWorkflowRepository::new(pool.clone())
        .find_snapshot_by_id(&context.workflow.id, &WorkflowSnapshotId::new(snapshot_id))
        .map_err(|error| BackendError::internal("failed to load target snapshot", error))?
        .ok_or_else(|| {
            BackendError::from(ApplicationError::WorkflowSnapshotNotFoundById {
                snapshot_id: snapshot_id.to_string(),
            })
        })?;
    let plan = check_switch(pool, &context, &target)?;
    apply_switch(
        pool,
        skills_root,
        workspace_root,
        &context,
        &target,
        plan,
        now,
    )?;
    Ok(true)
}

fn start_prerequisites_error(error: StartPrerequisitesError) -> ApplicationError {
    match error {
        StartPrerequisitesError::WorkflowSkillNotFound { skill_id } => {
            ApplicationError::WorkflowSkillNotFound { skill_id }
        }
        StartPrerequisitesError::WorkflowRoleNotFound { role_id } => {
            ApplicationError::WorkflowRoleNotFound { role_id }
        }
        StartPrerequisitesError::SkillMaterializationError { message } => {
            ApplicationError::WorkflowRunStartFailed { message }
        }
        StartPrerequisitesError::AgentSkillDeliveryUnsupported { agent_ref } => {
            ApplicationError::WorkflowRunStartFailed {
                message: format!("agent {agent_ref} does not support workflow-managed skills"),
            }
        }
        StartPrerequisitesError::AgentSkillDeliveryError { agent_ref, message } => {
            ApplicationError::WorkflowRunStartFailed {
                message: format!("failed to resolve skill delivery for {agent_ref}: {message}"),
            }
        }
        StartPrerequisitesError::Repository(source) => {
            ApplicationError::WorkflowRunRepository { source }
        }
    }
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
