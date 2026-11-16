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

/// A minimal standalone configuration whose agents would run as `workload_uid`.
fn standalone(root: &std::path::Path, workload_uid: Option<u32>) -> ServiceConfig {
    serde_json::from_value(serde_json::json!({
        "node": {
            "home_directory": root.join("node"),
            "identity": "Discover",
            "repositories": [],
        },
        "process": {
            "workload_uid": workload_uid,
            "git_program": "/usr/bin/git",
            "environment": {},
            "command_timeout_ms": 1000,
            "cleanup_timeout_ms": 1000,
            "shutdown_grace_ms": 1000,
        },
        "recovery_interval_ms": 50,
        "timezone": "UTC",
    }))
    .expect("service configuration")
}

/// With a separate workload user the Node refuses to start, before it opens anything, unless the
/// kernel stops that user from hard-linking files it does not own; the error names the sysctl.
#[tokio::test]
async fn a_separate_workload_requires_protected_hardlinks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sysctl = root.join("protected_hardlinks");
    for (value, found) in [
        (Some("0\n"), "it reads \"0\""),
        (Some("2\n"), "it reads \"2\""),
        (None, "it could not be read"),
    ] {
        match value {
            Some(value) => std::fs::write(&sysctl, value).expect("sysctl stand-in"),
            None => std::fs::remove_file(&sysctl).expect("no sysctl"),
        }
        let error =
            super::serve_on_host(standalone(root, Some(1000)), Shutdown::default(), &sysctl)
                .await
                .expect_err("the host is refused");
        let message = error.to_string();
        assert!(
            message.starts_with(&format!(
                "process.workload_uid requires the sysctl fs.protected_hardlinks = 1 ({}), but {found}",
                sysctl.display()
            )),
            "{message}"
        );
        assert!(!root.join("node").exists());
    }
}

/// The protection is read only for a separate workload, and `1` satisfies it.
#[test]
fn protected_hardlinks_is_required_only_for_a_separate_workload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (on, off) = (temp.path().join("on"), temp.path().join("off"));
    std::fs::write(&on, "1\n").expect("on");
    std::fs::write(&off, "0\n").expect("off");
    let shared = standalone(temp.path(), None);
    let separate = standalone(temp.path(), Some(1000));
    let checks: Vec<_> = [(&shared, &off), (&separate, &on)]
        .into_iter()
        .map(|(config, sysctl)| {
            super::host::require_protected_hardlinks(&config.process, sysctl)
                .map_err(|error| error.to_string())
        })
        .collect();
    assert_eq!(checks, vec![Ok(()), Ok(())]);
}
