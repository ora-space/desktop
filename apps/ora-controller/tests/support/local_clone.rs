//! Clone dispatches as an earlier Controller's local intake left them in the SQLite database.
//! Nothing registers local clones any more (a Workspace clone takes a Git Server snapshot input
//! that only Cloud can fix), so tests write the adapter's table directly to exercise what the
//! adapter still does with the dispatches it holds: reconcile, take over and report them.
use ora_node_protocol::*;
use std::path::Path;

/// The commit the seeded snapshot names.
pub const BASE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// A snapshot clone input for the Node `node`, valid under the Node protocol.
pub fn spec() -> CloneExecutionSpec {
    CloneExecutionSpec {
        node_id: NodeId::new("node"),
        repository_url: GitRepositoryUrl::parse(
            "https://git.ora.test/6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b/0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d.git",
        )
        .unwrap(),
        snapshot_ref: SnapshotRef::parse(&format!(
            "refs/ora/upstreams/9a8b7c6d-5e4f-4a3b-9c2d-1e0f9a8b7c6d/snapshots/{BASE_COMMIT}"
        ))
        .unwrap(),
        base_commit: CommitId::new(BASE_COMMIT),
        workspace_ref: WorkspaceRef::parse(
            "refs/heads/workspaces/4b3c2d1e-0f9a-4b8c-9d7e-6f5a4b3c2d1e/main",
        )
        .unwrap(),
    }
}

/// Writes one accepted dispatch without a result into the database under `home`, exactly as the
/// adapter's table holds it, and returns the command it stands for.
pub fn seed(home: &Path, request: &str) -> CloneRepositoryMessage {
    let command = CloneRepositoryMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: Some(RequestId::new(request)),
        operation_id: OperationId::new(format!("{request}-operation")),
        execution_id: ExecutionId::new(format!("{request}-execution")),
        payload: CloneRepository { spec: spec() },
    };
    command.validate().unwrap();
    rusqlite::Connection::open(home.join("ora-controller.sqlite3"))
        .unwrap()
        .execute(
            "INSERT INTO clone_operations VALUES (?1,?2,?3,?4,?5,NULL)",
            rusqlite::params![
                request,
                command.operation_id.as_str(),
                command.execution_id.as_str(),
                command.payload.spec.node_id.as_str(),
                serde_json::to_string(&command).unwrap()
            ],
        )
        .unwrap();
    command
}
