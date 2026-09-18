#![cfg(target_os = "linux")]

use ora_process_protocol::{DescendantPolicy, RunSpec};
use ora_process_runtime::{LinuxHelperConfig, spawn_linux_helper_workload};
use pretty_assertions::assert_eq;

/// The kernel's sentinel value must never turn a requested credential drop into a no-op.
#[test]
fn helper_rejects_unchanged_identity_sentinels() {
    for field in ["manager_uid", "workload_uid", "workload_gid"] {
        let mut config = serde_json::json!({
            "version":1, "manager_uid":1000, "workload_uid":2000,
            "workload_gid":2000, "cgroup_root":"/sys/fs/cgroup/ora-workloads"
        });
        config[field] = serde_json::json!(u32::MAX);
        assert!(LinuxHelperConfig::from_json(config.to_string().as_bytes()).is_err());
    }
}

/// Calling the production launch entry without deployment authority cannot execute a command.
#[test]
fn launch_rejects_untrusted_deployment_without_executing() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let marker = directory.path().join("executed");
    let mut spec = RunSpec::new("/bin/sh", directory.path(), DescendantPolicy::WaitForAll);
    spec.args = [
        "-c".into(),
        "printf executed > \"$1\"".into(),
        "test".into(),
        marker.clone().into(),
    ]
    .into();
    let result = spawn_linux_helper_workload(
        &directory.path().join("missing-config.json"),
        &directory.path().join("scope").join("run"),
        &spec,
    );
    assert!(result.is_err());
    assert_eq!(marker.exists(), false);
    Ok(())
}
