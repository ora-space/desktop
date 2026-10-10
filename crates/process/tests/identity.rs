#![cfg(target_os = "linux")]
//! Spawning under a dropped identity. This file is its own test binary so the child-process
//! census below is not disturbed by unrelated tests spawning children in parallel.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use ora_process::{
    ManagedProcess, ProcessIdentity, ProcessSpawner, ProcessSpec, TokioProcessSpawner,
};
use ora_utils::process::LinuxChildIdentity;
use pretty_assertions::assert_eq;
use tokio::io::{AsyncBufReadExt, BufReader};

/// The conventional unprivileged `nobody` identity.
const NOBODY: u32 = 65534;

/// Lists every live or zombie child of this test process, across all of its threads.
fn children() -> BTreeSet<String> {
    let mut children = BTreeSet::new();
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return children;
    };
    for task in tasks.flatten() {
        if let Ok(list) = std::fs::read_to_string(task.path().join("children")) {
            children.extend(list.split_whitespace().map(str::to_string));
        }
    }
    children
}

/// An unprivileged parent cannot take another identity, so the spawn must fail closed instead of
/// running the program as the parent, and the forked child must already be reaped.
#[tokio::test]
async fn an_identity_the_parent_cannot_take_fails_the_spawn() {
    // SAFETY: geteuid only reads the process identity.
    if unsafe { libc::geteuid() } == 0 {
        // Root can take the identity; the privileged test below covers that path.
        return;
    }
    let identity = LinuxChildIdentity::new(NOBODY, NOBODY).expect("nobody is a valid identity");
    let spawner = TokioProcessSpawner::running_as(ProcessIdentity::Linux(identity));
    let before = children();

    let error = spawner
        .spawn(ProcessSpec::new("/bin/sh").args(["-c", "exit 0"]))
        .expect_err("an unprivileged parent cannot drop to another identity");

    assert_eq!(
        (error.kind(), children()),
        (std::io::ErrorKind::PermissionDenied, before)
    );
}

/// Opt-in proof on a root host: the child and its descendants run as the identity with no
/// groups, no capabilities, `no_new_privs` and a private umask, and still die with the tree.
#[tokio::test]
#[ignore = "requires root; run with `sudo -E cargo test -p ora-process --test identity -- --ignored`"]
async fn a_root_parent_drops_the_whole_tree_to_the_identity() {
    let identity = LinuxChildIdentity::new(NOBODY, NOBODY).expect("nobody is a valid identity");
    let spawner = TokioProcessSpawner::running_as(ProcessIdentity::Linux(identity));
    let script = r#"
id -u
id -g
id -G
grep -E '^(NoNewPrivs|CapEff):' /proc/self/status | tr -s ' \t' ' '
umask
sleep 60 &
echo "$!"
wait
"#;
    let mut process = spawner
        .spawn(ProcessSpec::new("/bin/sh").args(["-c", script]).cwd("/"))
        .expect("root can drop to nobody");
    let stdout = process.take_stdout().expect("stdout pipe");
    let mut lines = BufReader::new(stdout).lines();
    let mut observed = Vec::new();
    while observed.len() < 7 {
        observed.push(
            lines
                .next_line()
                .await
                .expect("read stdout")
                .expect("the probe prints seven lines"),
        );
    }
    let descendant: i32 = observed.pop().expect("pid line").parse().expect("pid");

    assert_eq!(
        observed,
        vec![
            NOBODY.to_string(),
            NOBODY.to_string(),
            NOBODY.to_string(),
            "CapEff: 0000000000000000".to_string(),
            "NoNewPrivs: 1".to_string(),
            "0077".to_string(),
        ]
    );

    process.kill().await.expect("kill the tree");
    let _ = process.wait().await;
    let deadline = Instant::now() + Duration::from_secs(5);
    // SAFETY: kill with signal 0 only probes whether the process still exists.
    while unsafe { libc::kill(descendant, 0) } == 0 && !is_zombie(descendant) {
        assert!(
            Instant::now() < deadline,
            "the descendant outlived the tree kill"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A killed descendant is reparented and stays a zombie until its new parent reaps it.
fn is_zombie(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(')')
                .map(|(_, rest)| rest.trim_start().starts_with('Z'))
        })
        .unwrap_or(false)
}
