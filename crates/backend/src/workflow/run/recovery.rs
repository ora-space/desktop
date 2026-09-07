//! Best-effort recovery of workflow runs and their owned baseline side files.

use super::engine::{ConcreteWorkflowRunEngine, reconcile_running_workflow_runs};
use crate::clock::SystemClock;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{Clock, WorkflowRunEngineRepository};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_logging::ora_error;
use std::path::Path;
use std::sync::Arc;

/// Fails runs interrupted by a previous process, then reconciles the survivors.
///
/// Runs that were `Running` or `Failed` when the process died have their non-terminal node runs
/// marked `Failed` with `interrupted_by_restart`. Surviving `Running` runs are then reconciled:
/// stalled ones resume scheduling, and invalid `Pending` nodes fail closed. The sweep is
/// idempotent and best-effort so a storage failure cannot block startup.
pub(crate) fn run_workflow_run_boot_sweep(
    pool: &RepositoryPool,
    engine: &Arc<ConcreteWorkflowRunEngine>,
    run_locks: &Arc<KeyedResourceLocks>,
    clock: SystemClock,
) {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_ids = match repository.list_recoverable_runs() {
        Ok(run_ids) => run_ids,
        Err(error) => {
            ora_error!(error = %error, "workflow run boot sweep failed to list recoverable runs");
            return;
        }
    };
    if !run_ids.is_empty()
        && let Err(error) =
            repository.fail_orphaned_node_runs(&run_ids, clock.now_timestamp_millis())
    {
        ora_error!(error = %error, "workflow run boot sweep failed to fail orphaned node runs");
    }
    reconcile_running_workflow_runs(engine, run_locks, pool);
}

/// Deletes worktree-baseline side files whose node run is missing or no longer awaiting input.
///
/// Baselines exist only while an interactive node awaits input; a crash between a node's terminal
/// commit and its baseline deletion, or a node that failed without cleanup, leaves orphaned side
/// files that this sweep reclaims at the next boot.
pub(crate) fn prune_orphaned_baselines(pool: &RepositoryPool, baselines_root: &Path) {
    let Ok(entries) = std::fs::read_dir(baselines_root) else {
        return;
    };
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let node_run_id = ora_domain::WorkflowNodeRunId::new(name);
        let still_awaiting = repository
            .find_node_run_by_id(&node_run_id)
            .map(|node_run| {
                node_run.is_some_and(|node| node.status == ora_domain::WorkflowNodeStatus::Pending)
            })
            .unwrap_or(false);
        if !still_awaiting {
            let _ = std::fs::remove_file(&path);
        }
    }
}
