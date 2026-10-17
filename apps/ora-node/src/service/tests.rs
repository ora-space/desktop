use crate::{ServiceConfig, Shutdown};
use pretty_assertions::assert_eq;

/// A Cloud Node (one with a platform-assigned target) without the git-access identity refuses to
/// start with the bounded code, before it opens anything (Node delivery push ADR D7).
#[tokio::test]
async fn a_cloud_node_without_git_access_refuses_to_start() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let config: ServiceConfig = serde_json::from_value(serde_json::json!({
        "node": {
            "home_directory": root.join("node"),
            "identity": "Discover",
            "repositories": [],
        },
        "process": {
            "host_directory": root.join("host"),
            "expected_uid": 0,
            "git_program": "/usr/bin/git",
            "environment": {},
            "command_timeout_ms": 1000,
            "cleanup_timeout_ms": 1000,
            "shutdown_grace_ms": 1000,
        },
        "clone": {
            "repository_root": root.join("clones"),
            "git_config": root.join("clone.gitconfig"),
            "search_path": ["/usr/bin"],
        },
        "control": {
            "target": {
                "tenant_id": "tenant",
                "workspace_id": "workspace",
                "sandbox_id": "sandbox",
                "runtime_generation": 1,
            },
            "controller_id": "controller",
            "listen": { "kind": "ipc", "path": root.join("node").join("control.sock") },
            "heartbeat_ms": 100,
            "frame_timeout_ms": 1000,
        },
        "recovery_interval_ms": 50,
        "timezone": "UTC",
    }))
    .expect("service configuration");
    let error = crate::serve(config, Shutdown::default())
        .await
        .expect_err("the configuration is refused");
    assert_eq!(error.to_string(), "git_access_required");
    assert!(!root.join("node").exists());
}
