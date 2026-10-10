use crate::{AgentConfig, ProcessConfig, ServiceConfig, SessionWorkload, Shutdown};
use ora_process::ProcessIdentity;
use pretty_assertions::assert_eq;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A service configuration over `root` that would run agents as `workload_uid` from
/// `workload_directory`; everything else is valid and never reached by these tests.
fn config(
    root: &Path,
    workload_uid: Option<u32>,
    workload_directory: Option<PathBuf>,
) -> ServiceConfig {
    let mut config: ServiceConfig = serde_json::from_value(serde_json::json!({
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
        "recovery_interval_ms": 50,
        "timezone": "UTC",
    }))
    .expect("service configuration");
    config.process.workload_uid = workload_uid;
    config.agent = Some(AgentConfig {
        deno_path: PathBuf::from("/usr/bin/deno"),
        ready_timeout_ms: 1000,
        workload_directory,
    });
    config
}

/// Creates `path` as a directory with exactly `mode`.
fn directory(path: &Path, mode: u32) -> PathBuf {
    fs::create_dir_all(path).expect("create directory");
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set mode");
    path.to_path_buf()
}

/// Agents run as the workload user exactly when Git does, from a directory only the Node may
/// change that overlaps none of its state; `serve` refuses every other combination before it
/// opens anything.
#[tokio::test]
async fn serve_refuses_an_agent_workload_that_does_not_match_the_process_workload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let valid = directory(&root.join("agent"), 0o711);
    let shared = directory(&root.join("shared"), 0o777);
    let inside_home = directory(&root.join("node").join("agent"), 0o711);
    let cases = [
        config(root, Some(1000), None),
        config(root, None, Some(valid.clone())),
        config(root, Some(1000), Some(PathBuf::from("relative"))),
        config(root, Some(1000), Some(root.join("missing"))),
        config(root, Some(1000), Some(shared)),
        config(root, Some(1000), Some(inside_home)),
    ];

    let mut errors = Vec::new();
    for case in cases {
        errors.push(
            crate::serve(case, Shutdown::default())
                .await
                .expect_err("the configuration is refused")
                .to_string(),
        );
    }

    let mismatch = "agent workload_directory is required exactly when process.workload_uid is set";
    assert_eq!(
        errors,
        vec![
            mismatch.to_string(),
            mismatch.to_string(),
            "agent workload directory must be an absolute UTF-8 path without parent traversal"
                .to_string(),
            "No such file or directory (os error 2)".to_string(),
            "agent workload directory must belong to the Node identity and be writable by no one else"
                .to_string(),
            "agent workload directory overlaps Node state, host state or the clone root"
                .to_string(),
        ]
    );
}

/// Without a workload user agents share the Node identity; with one they take exactly that user,
/// with a group equal to it.
#[test]
fn the_workload_follows_the_process_workload_user() {
    let temp = tempfile::tempdir().expect("tempdir");
    let directory = temp.path().join("agent");
    let shared = config(temp.path(), None, None);
    let separate = config(temp.path(), Some(1000), Some(directory.clone()));
    let process = |config: &ServiceConfig| -> ProcessConfig { config.process.clone() };

    assert_eq!(
        (
            super::workload(shared.agent.as_ref(), &process(&shared)).expect("shared"),
            super::workload(separate.agent.as_ref(), &process(&separate)).expect("separate"),
        ),
        (
            SessionWorkload::Shared,
            SessionWorkload::Separate {
                directory,
                identity: ProcessIdentity::Linux(
                    ora_utils::process::LinuxChildIdentity::new(1000, 1000).expect("identity")
                ),
                uid: 1000,
                gid: 1000,
            },
        )
    );
}
