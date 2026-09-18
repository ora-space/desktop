use super::session::node;
use super::support::*;
use ora_node_protocol::*;
use serde_json::json;

#[path = "worktree/completed.rs"]
mod completed;

#[path = "worktree/fixtures.rs"]
mod fixtures;
#[path = "worktree/rejections.rs"]
mod rejections;

/// Returns one complete normalized Worktree execution input shared by command fixtures.
pub(super) fn spec() -> WorktreeExecutionSpec {
    WorktreeExecutionSpec {
        node_id: NodeId::new("node-1"),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        repository: RepositoryRef::new("repository-1"),
        main_workspace: MainWorkspaceBinding {
            workspace_id: WorkspaceId::new("workspace-main"),
            path: NodePath::new("/node/repos/ora"),
        },
        base_ref: GitRef::new("refs/heads/main"),
        expected_branch: BranchName::new("ora/12345678"),
        path_policy: WorktreePathPolicy::NodeManaged {
            directory_name: "workspace-task".to_string(),
        },
    }
}

/// Returns a successful creation result containing every Node-scoped fact.
pub(super) fn ready_result() -> WorktreeReady {
    WorktreeReady {
        node: node(),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        facts: WorktreeFacts {
            path: NodePath::new("/node/worktrees/workspace-task"),
            branch: BranchName::new("ora/12345678"),
            base_commit: CommitId::new("0123456789abcdef"),
        },
    }
}

/// Returns one structured creation failure.
pub(super) fn failed_result() -> WorktreeFailed {
    WorktreeFailed {
        node: node(),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        failure: WorktreeFailure {
            code: WorktreeFailureCode::BranchConflict,
            message: "branch is already checked out".to_string(),
        },
    }
}

/// Returns the idempotent already-absent removal outcome.
pub(super) fn removed_result() -> WorktreeRemoved {
    WorktreeRemoved {
        node: node(),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        outcome: WorktreeRemovalOutcome::AlreadyAbsent,
    }
}

/// Returns one structured removal failure.
pub(super) fn removal_failed_result() -> WorktreeRemovalFailed {
    WorktreeRemovalFailed {
        node: node(),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        failure: WorktreeFailure {
            code: WorktreeFailureCode::OperationFailed,
            message: "Git refused to remove the worktree".to_string(),
        },
    }
}

/// Proves fragmented I/O preserves every legal message in this business slice.
#[tokio::test]
async fn round_trips_messages() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_round_trip().await?;
    }
    Ok(())
}

/// Locks independent wire shapes, optional output fields, and extension acceptance.
#[tokio::test]
async fn preserves_wire_shapes() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_wire().await?;
        case.assert_extensions().await?;
    }
    Ok(())
}

/// Applies version and direction constraints to every legal shape in this slice.
#[tokio::test]
async fn rejects_versions_directions_and_payloads() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_envelope_rejections().await?;
    }
    Ok(())
}
