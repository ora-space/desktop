//! Handing the checkout to a separate workload user before the agent starts.
use super::support::*;
use ora_node_protocol::{AgentSessionEndReason, AgentSessionEnded};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;

/// Builds `base/d/d/…/d`, `depth` directories deep, with paths of at most three components:
/// each round moves the chain built so far into a fresh directory that then takes its name.
fn deep_chain(base: &Path, depth: usize) {
    let (top, next) = (base.join("d"), base.join("next"));
    fs::create_dir(&top).unwrap();
    for _ in 1..depth {
        fs::create_dir(&next).unwrap();
        fs::rename(&top, next.join("d")).unwrap();
        fs::rename(&next, &top).unwrap();
    }
}

/// Removes a chain built by [`deep_chain`] the same way round, so cleanup never builds a long
/// path nor recurses once per level.
fn remove_deep_chain(base: &Path) {
    let (top, next) = (base.join("d"), base.join("next"));
    while top.join("d").exists() {
        fs::rename(top.join("d"), &next).unwrap();
        fs::remove_dir(&top).unwrap();
        fs::rename(&next, &top).unwrap();
    }
    fs::remove_dir(&top).unwrap();
}

/// A checkout nested deeper than the hand-over walk descends is never handed to the workload
/// user: the session ends `agent_failed{checkout_too_deep}` before any plugin process or session
/// directory exists, and the Node keeps serving.
#[tokio::test(flavor = "multi_thread")]
async fn a_checkout_deeper_than_the_walk_bound_ends_the_session() {
    let fixture = Fixture::new();
    let base = fixture.checkout().join(".cache");
    fs::create_dir(&base).unwrap();
    deep_chain(&base, ora_utils::fs::MAX_OWN_TREE_DEPTH + 1);
    let sessions = fixture.sessions_with(PLUGIN_VERSION, fixture.separate_workload());

    fixture.start(&sessions, PLUGIN_VERSION, "[env]");
    let ended = fixture.ended().await;
    let leftovers = fs::read_dir(fixture.workload_directory()).unwrap().count();
    remove_deep_chain(&base);

    assert_eq!(
        (ended, leftovers, fixture.plugin_pids()),
        (refused_start(&fixture, "checkout_too_deep"), 0, Vec::new())
    );
    sessions.shutdown().await;
}

/// The terminal record a start refused before any plugin process with `detail`.
fn refused_start(fixture: &Fixture, detail: &str) -> AgentSessionEnded {
    AgentSessionEnded {
        node: fixture.node(),
        reason: AgentSessionEndReason::AgentFailed,
        detail: Some(detail.into()),
        plugin_id: None,
        setting_id: None,
        transport: None,
        scheduled_result: None,
        artifact_manifest: None,
    }
}

/// A hand-over that fails for any other reason — here a directory the walk may not list — ends
/// the session `agent_failed{agent_start_failed}`, never `checkout_too_deep`.
#[tokio::test(flavor = "multi_thread")]
async fn a_checkout_the_walk_cannot_list_ends_the_session_as_a_start_failure() {
    let fixture = Fixture::new();
    // Root may list anything, so the failure is only observable unprivileged.
    if std::os::unix::fs::MetadataExt::uid(&fs::metadata(fixture.checkout()).unwrap()) == 0 {
        return;
    }
    let locked = fixture.checkout().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let sessions = fixture.sessions_with(PLUGIN_VERSION, fixture.separate_workload());

    fixture.start(&sessions, PLUGIN_VERSION, "[env]");
    let ended = fixture.ended().await;
    fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();

    assert_eq!(
        (ended, fixture.plugin_pids()),
        (refused_start(&fixture, "agent_start_failed"), Vec::new())
    );
    sessions.shutdown().await;
}

/// A host that stopped protecting hard links after the Node started refuses the next start
/// before its walk: the session ends `agent_failed{agent_start_failed}` with no plugin process.
#[tokio::test(flavor = "multi_thread")]
async fn a_host_without_protected_hardlinks_refuses_the_start() {
    let fixture = Fixture::new();
    let sessions = fixture.sessions_with(PLUGIN_VERSION, fixture.separate_workload());
    fs::write(fixture.protected_hardlinks(), "0\n").unwrap();

    fixture.start(&sessions, PLUGIN_VERSION, "[env]");
    let ended = fixture.ended().await;

    assert_eq!(
        (ended, fixture.plugin_pids()),
        (refused_start(&fixture, "agent_start_failed"), Vec::new())
    );
    sessions.shutdown().await;
}
