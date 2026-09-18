#![cfg(target_os = "linux")]

use ora_process_runtime::LinuxHelperConfig;
use pretty_assertions::assert_eq;

/// Deployment configuration must not let workloads select or inherit management authority.
#[test]
fn helper_configuration_requires_separate_unprivileged_workload_identity() {
    let valid = br#"{"version":1,"manager_uid":1000,"workload_uid":2000,"workload_gid":2000,"cgroup_root":"/sys/fs/cgroup/ora-workloads"}"#;
    assert_eq!(LinuxHelperConfig::from_json(valid).map(|_| ()), Ok(()));
    for invalid in [
        r#"{"version":1,"manager_uid":1000,"workload_uid":1000,"workload_gid":2000,"cgroup_root":"/sys/fs/cgroup/ora-workloads"}"#,
        r#"{"version":1,"manager_uid":1000,"workload_uid":0,"workload_gid":2000,"cgroup_root":"/sys/fs/cgroup/ora-workloads"}"#,
        r#"{"version":1,"manager_uid":0,"workload_uid":2000,"workload_gid":2000,"cgroup_root":"/sys/fs/cgroup/ora-workloads"}"#,
        r#"{"version":1,"manager_uid":1000,"workload_uid":2000,"workload_gid":0,"cgroup_root":"/sys/fs/cgroup/ora-workloads"}"#,
        r#"{"version":2,"manager_uid":1000,"workload_uid":2000,"workload_gid":2000,"cgroup_root":"/sys/fs/cgroup/ora-workloads"}"#,
        r#"{"version":1,"manager_uid":1000,"workload_uid":2000,"workload_gid":2000,"cgroup_root":"/sys/fs/cgroup/ora-workloads","command":"arbitrary-root-command"}"#,
    ] {
        assert!(
            LinuxHelperConfig::from_json(invalid.as_bytes()).is_err(),
            "accepted {invalid}"
        );
    }
}

/// An administrator-owned ordinary directory is not evidence of a cgroup deployment.
#[test]
fn helper_rejects_a_non_cgroup_filesystem() {
    let config = LinuxHelperConfig::from_json(br#"{"version":1,"manager_uid":1000,"workload_uid":2000,"workload_gid":2000,"cgroup_root":"/"}"#)
        .unwrap_or_else(|error| panic!("parse: {error}"));
    assert_eq!(
        config.verify_deployment(),
        Err("workload root is not a cgroup v2 filesystem".into())
    );
}
