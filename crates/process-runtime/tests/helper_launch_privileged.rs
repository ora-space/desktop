#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use ora_process_protocol::{DescendantPolicy, RunId, RunSpec};
use ora_process_runtime::{check_linux_helper_deployment, spawn_linux_helper_workload};
use pretty_assertions::assert_eq;

/// Opt-in deployment proof: the workload observes dropped credentials and its assigned cgroup.
#[test]
#[ignore = "requires an explicitly provisioned root/cgroup v2 environment; see docs/process/linux/helper.md"]
fn launch_enters_cgroup_drops_authority_and_rejects_migration()
-> Result<(), Box<dyn std::error::Error>> {
    let config_path = PathBuf::from(std::env::var_os("ORA_PROCESS_HELPER_TEST_CONFIG").ok_or(
        "set ORA_PROCESS_HELPER_TEST_CONFIG to the administrator-owned test configuration",
    )?);
    check_linux_helper_deployment(&config_path)?;
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(&config_path)?)?;
    let root = Path::new(config["cgroup_root"].as_str().ok_or("cgroup root")?);
    let uid = config["workload_uid"].as_u64().ok_or("workload UID")?;
    let gid = config["workload_gid"].as_u64().ok_or("workload GID")?;
    let scope = root.join(RunId::new().to_string());
    // Only newly created directories are owned by this fixture; no existing subtree is adopted.
    std::fs::create_dir(&scope)?;
    let mut fixture = OwnedCgroups {
        scope,
        runs: Vec::new(),
    };
    std::fs::set_permissions(
        &fixture.scope,
        std::fs::Permissions::from_mode(/*mode*/ 0o755),
    )?;
    for _ in 0..2 {
        let path = fixture.scope.join(RunId::new().to_string());
        std::fs::create_dir(&path)?;
        fixture.runs.push(path.clone());
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(/*mode*/ 0o755))?;
    }
    let run = &fixture.runs[0];
    let other_run = &fixture.runs[1];
    let descriptor = File::open("/dev/null")?;
    // SAFETY: this deliberately inheritable duplicate belongs to the fixture, not another user.
    let inherited = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_DUPFD, 200) };
    if inherited < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: fcntl returned a fresh descriptor owned exactly once by the fixture.
    let _inherited = unsafe { OwnedFd::from_raw_fd(inherited) };
    let mut spec = RunSpec::new("/bin/sh", "/", DescendantPolicy::WaitForAll);
    spec.env.insert("ONLY_EXPLICIT".into(), "present".into());
    spec.args = [
        "-c".into(),
        r#"
test "$ONLY_EXPLICIT" = present || exit 90
test "${HOME+x}" != x || exit 91
test ! -e "/proc/self/fd/$3" || exit 92
if (printf 0 > "$1/cgroup.procs") 2>/dev/null; then exit 93; fi
if (printf 0 > "$2/cgroup.procs") 2>/dev/null; then exit 94; fi
/bin/cat /proc/self/status /proc/self/cgroup
"#
        .into(),
        "probe".into(),
        other_run.as_os_str().to_owned(),
        fixture.scope.as_os_str().to_owned(),
        inherited.to_string().into(),
    ]
    .into();
    let output = spawn_linux_helper_workload(&config_path, run, &spec)?.wait_with_output()?;
    assert_eq!((output.status.code(), output.stderr), (Some(0), Vec::new()));
    let text = String::from_utf8(output.stdout)?;
    let fields: BTreeMap<_, _> = text
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| {
            [
                "Uid",
                "Gid",
                "Groups",
                "CapInh",
                "CapPrm",
                "CapEff",
                "CapAmb",
                "NoNewPrivs",
            ]
            .contains(key)
        })
        .map(|(key, value)| (key, value.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect();
    assert_eq!(
        fields,
        BTreeMap::from([
            ("Uid", format!("{uid} {uid} {uid} {uid}")),
            ("Gid", format!("{gid} {gid} {gid} {gid}")),
            ("Groups", String::new()),
            ("CapInh", "0000000000000000".into()),
            ("CapPrm", "0000000000000000".into()),
            ("CapEff", "0000000000000000".into()),
            ("CapAmb", "0000000000000000".into()),
            ("NoNewPrivs", "1".into()),
        ])
    );
    let membership = text
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or("membership")?;
    assert!(Path::new(membership).ends_with(run.strip_prefix(root)?));

    // Workload cwd access must be checked after dropping root, not inherited from a root chdir.
    let private_cwd = tempfile::tempdir()?;
    std::fs::set_permissions(
        private_cwd.path(),
        std::fs::Permissions::from_mode(/*mode*/ 0o700),
    )?;
    spec.cwd = private_cwd.path().to_owned();
    assert!(spawn_linux_helper_workload(&config_path, run, &spec).is_err());
    for invalid in [root, fixture.scope.as_path(), private_cwd.path()] {
        assert!(spawn_linux_helper_workload(&config_path, invalid, &spec).is_err());
    }
    Ok(())
}

struct OwnedCgroups {
    scope: PathBuf,
    runs: Vec<PathBuf>,
}

impl Drop for OwnedCgroups {
    /// Failure cleanup targets only this fixture's freshly created scope, never the configured root.
    fn drop(&mut self) {
        if let Ok(mut kill) = OpenOptions::new()
            .write(/*write*/ true)
            .open(self.scope.join("cgroup.kill"))
        {
            let _ = kill.write_all(b"1");
        }
        for run in &self.runs {
            let _ = std::fs::remove_dir(run);
        }
        let _ = std::fs::remove_dir(&self.scope);
    }
}
