//! Re-validates a frozen run's declared dependencies before scheduling starts.
//!
//! A run freezes its graph, and the skill placements it deploys with, when it is created. Nothing
//! in that frozen state notices a dependency that disappears afterwards: uninstalling the plugin
//! that provided a bound Skill removes its catalog row and its package, but the run still holds a
//! receipt naming a worktree path that will never be materialized again. Starting such a run would
//! dispatch a prompt that says `/skill` while the package is absent, so the node silently runs
//! without the declared capability.
//!
//! Every operation that begins scheduling therefore re-resolves the graph's declared dependencies
//! against the live catalog and plugin host first, and refuses to start with the node and the
//! dependency named. The frozen receipt itself stays authoritative for execution: this module only
//! proves that its dependencies are still installable, so re-installing the dependency makes the
//! same run startable again without rewriting the run.
//!
//! Agent availability is deliberately not part of this check. Which agents exist depends on
//! installed plugins, and the runtime resolves the frozen agent identity when it starts the node
//! session and fails closed with `agent_not_installed`; rejecting an identity here would also
//! reject the portable identities the runtime deliberately accepts.

use super::prerequisites::{plugin_initializer, validate_mcp_bindings};
use crate::agent_runtime::AgentRuntimeManager;
use crate::error::BackendError;
use ora_application::{
    ApplicationError, ExecutionContext, WorkflowGraph, WorkflowRunEngineRepository,
    WorkflowRunWorkspaceInitializer,
};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{WorkflowRunId, WorkflowRunStatus};
use std::path::Path;
use std::sync::Arc;

/// Which scheduling operation the dependency precheck guards.
///
/// The gate follows the transition the engine will attempt: a start that is already a no-op must
/// stay a no-op instead of turning into a dependency error, while a restart and a resume really do
/// schedule again and must never skip validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StartOperation {
    /// A `Pending` run begins scheduling; every other status leaves the run unchanged.
    Start,
    /// Any non-`Running` run is reset and re-executes from its Start node.
    Restart,
    /// A `Failed`/`Cancelled` run re-executes the nodes that had not succeeded.
    Resume,
}

impl StartOperation {
    /// Reports whether the operation actually schedules work in this run status.
    fn schedules(self, status: WorkflowRunStatus) -> bool {
        match self {
            Self::Start => status == WorkflowRunStatus::Pending,
            Self::Restart => status != WorkflowRunStatus::Running,
            Self::Resume => {
                matches!(
                    status,
                    WorkflowRunStatus::Failed | WorkflowRunStatus::Cancelled
                )
            }
        }
    }
}

/// The live sources every dependency precheck resolves against.
///
/// All three travel together because they answer one question: what is installed right now. Passing
/// them as one value keeps the scheduling entry points from threading three positional parameters
/// through each layer.
#[derive(Clone, Copy)]
pub(super) struct DependencySources<'a> {
    pub pool: &'a RepositoryPool,
    pub skills_root: &'a Path,
    pub plugin_api: &'a Arc<crate::plugin::PluginApi>,
}

impl<'a> DependencySources<'a> {
    /// Groups the repository, skill catalog root, and plugin host one precheck reads.
    pub(super) fn new(
        pool: &'a RepositoryPool,
        skills_root: &'a Path,
        plugin_api: &'a Arc<crate::plugin::PluginApi>,
    ) -> Self {
        Self {
            pool,
            skills_root,
            plugin_api,
        }
    }
}

/// Refuses to begin scheduling when a dependency the frozen graph declares is no longer available.
///
/// A run that will not schedule keeps its current state and reports no error, so idempotent
/// commands (starting an already running run, restarting one that is still running) still answer
/// with the run instead of a dependency failure.
pub(super) fn ensure_dependencies_available(
    dependencies: DependencySources<'_>,
    agent_runtime: &AgentRuntimeManager,
    run_id: &WorkflowRunId,
    operation: StartOperation,
) -> Result<(), BackendError> {
    let context = load_context(dependencies.pool, run_id)?;
    if !operation.schedules(context.run.status) {
        return Ok(());
    }
    let workspace_root = agent_runtime
        .workspace_cwd(&context.run.workspace_id)
        .map_err(BackendError::from)?;
    let graph = WorkflowGraph::parse(&context.graph_json).map_err(ApplicationError::from)?;
    let initializer = plugin_initializer(
        dependencies.skills_root,
        dependencies.pool,
        dependencies.plugin_api,
    )
    .map_err(|error| ApplicationError::WorkflowRunStartFailed {
        message: error.to_string(),
    })?;
    // Skills and roles first: they are the deploy dependencies the frozen receipt addresses.
    initializer
        .initialize_workspace(&graph, &workspace_root)
        .and_then(|_receipt| validate_mcp_bindings(dependencies.plugin_api, &graph))
        .map_err(ApplicationError::from_start_prerequisites_error)?;
    Ok(())
}

/// Reads the run's frozen graph and workspace, or reports the run as missing.
fn load_context(
    pool: &RepositoryPool,
    run_id: &WorkflowRunId,
) -> Result<ExecutionContext, BackendError> {
    SqliteWorkflowRunEngineRepository::new(pool.clone())
        .find_execution_context(run_id)
        .map_err(|error| BackendError::internal("failed to load workflow run dependencies", error))?
        .ok_or_else(|| {
            BackendError::from(ApplicationError::WorkflowRunNotFound {
                run_id: run_id.to_string(),
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// A start only schedules when the run is `Pending`; every other status is an idempotent
    /// answer, so a dependency that disappeared must not turn it into an error.
    #[test]
    fn start_operation_only_validates_a_pending_run() {
        assert_eq!(
            [
                WorkflowRunStatus::Pending,
                WorkflowRunStatus::Running,
                WorkflowRunStatus::Succeeded,
                WorkflowRunStatus::Failed,
                WorkflowRunStatus::Cancelled,
            ]
            .map(|status| StartOperation::Start.schedules(status)),
            [true, false, false, false, false]
        );
    }

    /// A restart resets every non-running status and therefore always re-executes.
    #[test]
    fn restart_operation_validates_every_resettable_status() {
        assert_eq!(
            [
                WorkflowRunStatus::Pending,
                WorkflowRunStatus::Running,
                WorkflowRunStatus::Succeeded,
                WorkflowRunStatus::Failed,
                WorkflowRunStatus::Cancelled,
            ]
            .map(|status| StartOperation::Restart.schedules(status)),
            [true, false, true, true, true]
        );
    }

    /// Only a failed or cancelled run resumes; the other statuses are reported as not resumable.
    #[test]
    fn resume_operation_only_validates_resumable_statuses() {
        assert_eq!(
            [
                WorkflowRunStatus::Pending,
                WorkflowRunStatus::Running,
                WorkflowRunStatus::Succeeded,
                WorkflowRunStatus::Failed,
                WorkflowRunStatus::Cancelled,
            ]
            .map(|status| StartOperation::Resume.schedules(status)),
            [false, false, false, true, true]
        );
    }
}
