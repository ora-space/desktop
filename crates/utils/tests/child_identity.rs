#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::Command;

use ora_utils::process::LinuxChildIdentity;
use pretty_assertions::assert_eq;

/// Sentinel IDs must not silently preserve a root identity during a credential transition.
#[test]
fn rejects_root_and_unchanged_identity_sentinels() {
    for (uid, gid) in [(0, 2000), (2000, 0), (u32::MAX, 2000), (2000, u32::MAX)] {
        assert_eq!(
            LinuxChildIdentity::new(uid, gid)
                .err()
                .map(|error| error.raw_os_error()),
            Some(Some(libc::EINVAL))
        );
    }
}

/// A failed privilege transition must fail spawn, not execute the business command unchanged.
#[test]
fn unprivileged_transition_failure_prevents_execution() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: querying this test process's effective UID has no side effects.
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "requires an unprivileged runner"
    );
    let directory = tempfile::tempdir()?;
    let marker = directory.path().join("executed");
    let identity = LinuxChildIdentity::new(/*uid*/ 2000, /*gid*/ 2000)?;
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "printf executed > \"$1\"", "test"])
        .arg(&marker);
    // SAFETY: the transition runs only in the forked child and returns immediately on failure.
    unsafe {
        command.pre_exec(move || identity.enter_child());
    }
    assert!(command.output().is_err());
    assert_eq!(marker.exists(), false);
    Ok(())
}
