#![cfg(unix)]
//! Handing one descriptor to one child. An advisory lock on the descriptor's open file makes
//! inheritance observable from outside: the lock stays held for as long as any process still
//! has the descriptor open, whatever its identity or process group.

use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;

use ora_process::{
    ManagedProcess, ProcessIdentity, ProcessSpawner, ProcessSpec, TokioProcessSpawner,
};
use pretty_assertions::assert_eq;

/// Opens `path` and takes its exclusive lock, as the single holder a child may inherit from.
fn locked(path: &Path) -> Arc<OwnedFd> {
    let file = File::create(path).expect("create the lock file");
    // SAFETY: flock only operates on the descriptor this test just opened.
    assert_eq!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    Arc::new(OwnedFd::from(file))
}

/// Whether any process still holds the descriptor that locked `path`.
fn held(path: &Path) -> bool {
    let probe = File::open(path).expect("reopen the lock file");
    // SAFETY: flock only operates on the probe's own descriptor, closed when it drops.
    unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) != 0 }
}

/// A child that outlives the test unless its tree is killed.
fn sleeper() -> ProcessSpec {
    ProcessSpec::new("/bin/sh").args(["-c", "exec sleep 60"])
}

/// Only the child whose spec names the descriptor keeps it: a sibling spawned while the parent
/// still had it open does not, so the lock follows exactly one process tree.
#[tokio::test]
async fn only_the_named_child_inherits_the_descriptor() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("lock");
    let descriptor = locked(&path);
    let spawner = TokioProcessSpawner::new();

    let bystander = spawner.spawn(sleeper()).expect("spawn the bystander");
    let holder = spawner
        .spawn(sleeper().inherit_descriptor(descriptor.clone()))
        .expect("spawn the holder");
    drop(descriptor);
    let while_holder_runs = held(&path);

    holder.kill().await.expect("kill the holder");
    holder.wait().await.expect("reap the holder");
    let after_holder_exit = held(&path);
    bystander.kill().await.expect("kill the bystander");
    bystander.wait().await.expect("reap the bystander");

    assert_eq!((while_holder_runs, after_holder_exit), (true, false));
}

/// Opt-in proof on a root host: the identity drop marks every descriptor close-on-exec, and the
/// named one still reaches the child, which could not have opened the file itself.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires root; run with `sudo -E cargo test -p ora-process --test descriptor -- --ignored`"]
async fn a_dropped_identity_keeps_only_the_named_descriptor() {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncReadExt;

    let directory = tempfile::tempdir().expect("tempdir");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("make the directory private");
    let path = directory.path().join("lock");
    let descriptor = locked(&path);
    let other = File::open("/dev/null").expect("open another descriptor");
    let identity =
        ora_utils::process::LinuxChildIdentity::new(/*uid*/ 65534, /*gid*/ 65534)
            .expect("nobody is a valid identity");
    let spawner = TokioProcessSpawner::running_as(ProcessIdentity::Linux(identity));
    // The child reports which of the two descriptors it received, then whether it can open the
    // lock file on its own, and stays alive so the lock can be probed.
    let script = format!(
        "test -e /proc/self/fd/{kept}; echo $?; test -e /proc/self/fd/{dropped}; echo $?; \
         cat '{path}' 2>/dev/null; echo $?; exec sleep 60",
        kept = descriptor.as_raw_fd(),
        dropped = other.as_raw_fd(),
        path = path.display(),
    );

    let mut child = spawner
        .spawn(
            ProcessSpec::new("/bin/sh")
                .args(["-c", script.as_str()])
                .cwd("/")
                .inherit_descriptor(descriptor.clone()),
        )
        .expect("root can drop to nobody");
    drop(descriptor);
    let mut stdout = child.take_stdout().expect("stdout pipe");
    let mut report = [0_u8; 6];
    stdout
        .read_exact(&mut report)
        .await
        .expect("read the report");
    let while_child_runs = held(&path);
    child.kill().await.expect("kill the child");
    child.wait().await.expect("reap the child");

    assert_eq!(
        (
            String::from_utf8_lossy(&report).into_owned(),
            while_child_runs,
            held(&path)
        ),
        ("0\n1\n1\n".to_string(), true, false)
    );
}
