#![cfg(unix)]
//! Which children a host's reaper takes. This file is its own test binary because the reaper
//! client is process-global and can be started only once.

use std::time::Duration;

use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, TokioProcessSpawner};
use pretty_assertions::assert_eq;

/// A child that outlives its handle and the test unless something kills its tree.
fn sleeper() -> ProcessSpec {
    ProcessSpec::new("/bin/sh")
        .args(["-c", "exec sleep 60"])
        .keep_alive_on_drop()
}

/// Final reaper cleanup ends the children registered with it and leaves alone the children of a
/// spawner that opted out, so a host can scope the reaper to one class of its processes.
#[tokio::test]
async fn a_spawner_can_keep_its_children_out_of_the_reaper() {
    ora_process::initialize_reaper(env!("CARGO_BIN_EXE_ora-reaper")).expect("start the reaper");
    let registered = TokioProcessSpawner::new()
        .spawn(sleeper())
        .expect("spawn the registered child");
    let excluded = TokioProcessSpawner::new()
        .without_reaper_registration()
        .spawn(sleeper())
        .expect("spawn the excluded child");

    ora_process::shutdown_reaper().expect("shut the reaper down");
    let registered_ended = tokio::time::timeout(Duration::from_secs(5), registered.wait())
        .await
        .is_ok();
    let excluded_running = excluded
        .try_wait()
        .expect("poll the excluded child")
        .is_none();
    excluded.kill().await.expect("kill the excluded child");
    excluded.wait().await.expect("reap the excluded child");

    assert_eq!((registered_ended, excluded_running), (true, true));
}
