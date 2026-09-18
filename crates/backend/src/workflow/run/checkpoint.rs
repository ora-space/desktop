use crate::agent_runtime::AgentRuntimeManager;
use gitlancer::git::worktree::FindWorktreeRequest;
use gitlancer::{
    ChangeStatus, ChangedSinceRequest, CliGitRunner, Git, RepoRoot, SnapshotWorktreeRequest,
};
use ora_application::{FileChange, NodeFailure, WorkflowRunCallback, WorkflowRunEngineRepository};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{WorkflowNodeRunId, WorkflowRunId, WorkspaceId};
use ora_logging::ora_warn;
use std::path::Path;
use std::sync::Arc;

/// Result of attempting a pre-node git checkpoint. Never an error: provenance degrades in place.
pub(super) struct CheckpointOutcome {
    pub commit_oid: Option<String>,
    pub error: Option<String>,
}

/// Snapshots the workspace worktree into `refs/ora/checkpoints/<node_run_id>` before the node runs.
///
/// Any failure (not a git repo, git missing, command error) is recorded as `error` so the node
/// still starts. This is provenance, not a prerequisite.
pub(super) fn take_checkpoint(
    workspace_root: &Path,
    run_id: &WorkflowRunId,
    node_id: &str,
    node_run_id: &WorkflowNodeRunId,
) -> CheckpointOutcome {
    match take_checkpoint_inner(workspace_root, run_id, node_id, node_run_id) {
        Ok(commit_oid) => CheckpointOutcome {
            commit_oid: Some(commit_oid),
            error: None,
        },
        Err(error) => {
            ora_warn!(
                error = %error,
                workspace_root = %workspace_root.display(),
                run_id = %run_id,
                node_id = node_id,
                node_run_id = %node_run_id,
                "failed to take workflow node git checkpoint"
            );
            CheckpointOutcome {
                commit_oid: None,
                error: Some(error.to_string()),
            }
        }
    }
}

/// Lists files changed since a checkpoint, or an empty vec when git cannot produce a diff.
pub(super) fn changes_since_checkpoint(workspace_root: &Path, commit_oid: &str) -> Vec<FileChange> {
    match changes_since_checkpoint_inner(workspace_root, commit_oid) {
        Ok(changes) => changes,
        Err(error) => {
            ora_warn!(
                error = %error,
                workspace_root = %workspace_root.display(),
                commit_oid = commit_oid,
                "failed to compute file changes since workflow node git checkpoint"
            );
            Vec::new()
        }
    }
}

/// Records a pre-node checkpoint onto the node-run row. Repository errors still fail the node.
pub(super) fn record_pre_node_checkpoint(
    repository: &SqliteWorkflowRunEngineRepository,
    workspace_root: &Path,
    run_id: &WorkflowRunId,
    node_id: &str,
    node_run_id: &WorkflowNodeRunId,
    snapshot_id: &str,
    now: i64,
) -> Result<(), ora_application::RepositoryError> {
    let checkpoint = take_checkpoint(workspace_root, run_id, node_id, node_run_id);
    repository.record_node_checkpoint(
        node_run_id,
        snapshot_id,
        checkpoint.commit_oid.as_deref(),
        checkpoint.error.as_deref(),
        now,
    )
}

/// Attaches `payload.file_changes` derived from the node's stored checkpoint, when one exists.
pub(super) fn enrich_failure_with_changes(
    repository: &SqliteWorkflowRunEngineRepository,
    workspace_root: Option<&Path>,
    node_run_id: &WorkflowNodeRunId,
    failure: NodeFailure,
) -> NodeFailure {
    let node_run = match repository.find_node_run_by_id(node_run_id) {
        Ok(Some(node_run)) => node_run,
        Ok(None) => return failure,
        Err(error) => {
            ora_warn!(
                error = %error,
                node_run_id = %node_run_id,
                "failed to load node run while enriching failure file changes"
            );
            return failure;
        }
    };
    let Some(commit_oid) = checkpoint_oid_from_payload(node_run.payload.as_deref()) else {
        return failure;
    };
    let Some(workspace_root) = workspace_root else {
        ora_warn!(
            node_run_id = %node_run_id,
            "failed to resolve workspace cwd while enriching node failure file changes"
        );
        return failure;
    };
    failure.with_file_changes(changes_since_checkpoint(workspace_root, &commit_oid))
}

/// Reports a driven-node failure after attaching checkpoint-derived file changes on the blocking pool.
pub(super) async fn fail_dispatched_node(
    callback: Arc<dyn WorkflowRunCallback>,
    pool: RepositoryPool,
    agent_runtime: &AgentRuntimeManager,
    workspace_id: &WorkspaceId,
    run_id: WorkflowRunId,
    node_run_id: WorkflowNodeRunId,
    failure: NodeFailure,
) {
    let workspace_root = match agent_runtime.workspace_cwd(workspace_id) {
        Ok(root) => Some(root),
        Err(error) => {
            ora_warn!(
                error = %error,
                workspace_id = %workspace_id,
                "failed to resolve workspace cwd while enriching node failure file changes"
            );
            None
        }
    };
    let join = tokio::task::spawn_blocking(move || {
        let repository = SqliteWorkflowRunEngineRepository::new(pool);
        let failure = enrich_failure_with_changes(
            &repository,
            workspace_root.as_deref(),
            &node_run_id,
            failure,
        );
        callback.fail_node(&run_id, &node_run_id, failure);
    });
    if let Err(source) = join.await {
        ora_warn!("workflow node failure callback panicked: {source}");
    }
}

/// Discovers the workspace repository and writes a named checkpoint commit.
fn take_checkpoint_inner(
    workspace_root: &Path,
    run_id: &WorkflowRunId,
    node_id: &str,
    node_run_id: &WorkflowNodeRunId,
) -> Result<String, gitlancer::GitlancerError> {
    let git = Git::new(CliGitRunner);
    let repository = git.discover_repository(RepoRoot::new(workspace_root))?;
    let worktree = git.find_worktree(FindWorktreeRequest {
        repository: &repository,
        candidate_path: workspace_root,
    })?;
    let response = git.snapshot_worktree(SnapshotWorktreeRequest {
        worktree: &worktree,
        name: node_run_id.as_ref(),
        message: &format!("ora checkpoint: run {run_id} node {node_id}"),
    })?;
    Ok(response.commit_oid)
}

/// Maps a checkpoint diff onto the node payload's `file_changes` shape.
fn changes_since_checkpoint_inner(
    workspace_root: &Path,
    commit_oid: &str,
) -> Result<Vec<FileChange>, gitlancer::GitlancerError> {
    let git = Git::new(CliGitRunner);
    let repository = git.discover_repository(RepoRoot::new(workspace_root))?;
    let worktree = git.find_worktree(FindWorktreeRequest {
        repository: &repository,
        candidate_path: workspace_root,
    })?;
    let response = git.changed_since(ChangedSinceRequest {
        worktree: &worktree,
        commit_oid,
    })?;
    Ok(response
        .entries
        .into_iter()
        .map(|entry| {
            let path = match entry.status {
                ChangeStatus::Renamed { .. }
                | ChangeStatus::Added
                | ChangeStatus::Modified
                | ChangeStatus::Deleted
                | ChangeStatus::Other(_) => entry.path,
            };
            FileChange {
                path,
                additions: entry.additions.unwrap_or(0),
                deletions: entry.deletions.unwrap_or(0),
            }
        })
        .collect())
}

/// Reads `payload.checkpoint` when it is a non-null string.
fn checkpoint_oid_from_payload(payload: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload?).ok()?;
    value.get("checkpoint")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::{changes_since_checkpoint, take_checkpoint};
    use ora_domain::{WorkflowNodeRunId, WorkflowRunId};
    use ora_logging::with_trace_logging;
    use pretty_assertions::assert_eq;

    /// A plain directory is not a git repo, so the checkpoint degrades to an error string.
    #[test]
    fn take_checkpoint_on_a_plain_directory_records_an_error() {
        with_trace_logging(|| {
            let temp = tempfile::TempDir::new().unwrap();
            let outcome = take_checkpoint(
                temp.path(),
                &WorkflowRunId::new("run-1"),
                "review",
                &WorkflowNodeRunId::new("nr-1"),
            );
            assert_eq!(outcome.commit_oid, None);
            assert!(outcome.error.is_some());
        });
    }

    /// A real repository records a commit oid and a namespaced checkpoint ref.
    #[test]
    fn take_checkpoint_on_a_git_repository_writes_the_ref() {
        with_trace_logging(|| {
            let scaffold = ora_test_support::GitTestScaffold::new("backend-take-checkpoint")
                .expect("create Git test scaffold");
            scaffold
                .write_file(scaffold.repo_path(), "README.md", "seed\n")
                .expect("write seed");
            scaffold
                .stage_all_and_commit("chore: seed")
                .expect("commit seed");
            let node_run_id = WorkflowNodeRunId::new("nr-git");
            let outcome = take_checkpoint(
                scaffold.repo_path(),
                &WorkflowRunId::new("run-1"),
                "review",
                &node_run_id,
            );
            let oid = outcome.commit_oid.expect("checkpoint oid");
            assert_eq!(outcome.error, None);
            let resolved = scaffold
                .run_git(["rev-parse", &format!("refs/ora/checkpoints/{node_run_id}")])
                .expect("resolve checkpoint ref");
            assert_eq!(resolved.trim(), oid);
        });
    }

    /// Editing a tracked file after the checkpoint yields one file change with line counts.
    #[test]
    fn changes_since_checkpoint_reports_an_edited_file() {
        with_trace_logging(|| {
            let scaffold = ora_test_support::GitTestScaffold::new("backend-changes-since")
                .expect("create Git test scaffold");
            scaffold
                .write_file(scaffold.repo_path(), "src/a.ts", "one\n")
                .expect("write file");
            scaffold
                .stage_all_and_commit("chore: seed")
                .expect("commit seed");
            let outcome = take_checkpoint(
                scaffold.repo_path(),
                &WorkflowRunId::new("run-1"),
                "review",
                &WorkflowNodeRunId::new("nr-edit"),
            );
            let oid = outcome.commit_oid.expect("checkpoint oid");
            scaffold
                .write_file(scaffold.repo_path(), "src/a.ts", "one\ntwo\n")
                .expect("edit file");
            assert_eq!(
                changes_since_checkpoint(scaffold.repo_path(), &oid),
                vec![ora_application::FileChange {
                    path: "src/a.ts".to_string(),
                    additions: 1,
                    deletions: 0,
                }]
            );
        });
    }
}
