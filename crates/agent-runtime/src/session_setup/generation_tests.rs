//! A same-content resubmission must cross ACP independently before a prompt can use it.
use super::{
    LiveMcpEvent, LiveMcpPromptAdmission, LiveMcpState, SessionMcpMemberRevision,
    SessionMcpRevision, SessionMcpTransportKind,
};
use ora_domain::PluginId;
use pretty_assertions::assert_eq;
use semver::Version;

/// The member identity deliberately never changes: only the Controller's generation advances.
fn revision(generation: u64) -> SessionMcpRevision {
    SessionMcpRevision::for_generation(
        generation,
        vec![SessionMcpMemberRevision {
            plugin_id: PluginId::new("official", "test.mcp").unwrap(),
            package_version: Version::new(1, 2, 0),
            configuration_revision: 7,
            transport: SessionMcpTransportKind::Stdio,
        }],
    )
}

/// Default construction preserves the equality used by hosts which have no generation stream.
#[test]
fn legacy_revision_is_generation_zero() {
    let explicit = revision(0);
    assert_eq!(
        SessionMcpRevision::new(explicit.members().to_vec()),
        explicit
    );
    assert_eq!(
        SessionMcpRevision::default(),
        SessionMcpRevision::for_generation(0, vec![])
    );
}

/// Receiving the same binding again with a new target cannot reuse an earlier ACP response.
#[test]
fn new_generation_with_unchanged_members_waits_for_its_own_success() {
    let active = revision(0);
    let desired = revision(1);
    assert_eq!(active.members(), desired.members());
    let (pending, refresh) = LiveMcpState::Active(active.clone())
        .on_event(LiveMcpEvent::DesiredObserved(desired.clone()));
    assert!(refresh);
    assert_eq!(
        pending.prompt_admission(&desired),
        LiveMcpPromptAdmission::RefreshFirst {
            desired: desired.clone()
        }
    );
    let (sending, _) = pending.on_event(LiveMcpEvent::RefreshStarted(desired.clone()));
    assert_eq!(
        sending.on_event(LiveMcpEvent::RefreshSucceeded(active)),
        (sending.clone(), false)
    );
    let (current, again) = sending.on_event(LiveMcpEvent::RefreshSucceeded(desired.clone()));
    assert!(!again);
    assert_eq!(
        current.prompt_admission(&desired),
        LiveMcpPromptAdmission::Admit
    );
}

/// A newer target wins even when the old load completes successfully and its members are equal.
#[test]
fn in_flight_success_keeps_latest_generation_pending() {
    let (sending, _) =
        LiveMcpState::Active(revision(0)).on_event(LiveMcpEvent::RefreshStarted(revision(1)));
    let (newer, _) = sending.on_event(LiveMcpEvent::DesiredObserved(revision(2)));
    let (latest, _) = newer.on_event(LiveMcpEvent::DesiredObserved(revision(3)));
    let (pending, refresh) = latest.on_event(LiveMcpEvent::RefreshSucceeded(revision(1)));
    assert_eq!(
        (pending.clone(), refresh),
        (
            LiveMcpState::RefreshPending {
                active: revision(1),
                desired: revision(3)
            },
            true
        )
    );
    assert_eq!(
        pending.prompt_admission(&revision(3)),
        LiveMcpPromptAdmission::RefreshFirst {
            desired: revision(3)
        }
    );
    assert_eq!(
        pending.on_event(LiveMcpEvent::RefreshSucceeded(revision(2))),
        (pending, false)
    );
}

/// Failed old work blocks the latest generation; an unrelated response cannot clear the gate.
#[test]
fn old_failure_and_late_success_cannot_release_a_new_target() {
    let (sending, _) =
        LiveMcpState::Active(revision(0)).on_event(LiveMcpEvent::RefreshStarted(revision(1)));
    let (latest, _) = sending.on_event(LiveMcpEvent::DesiredObserved(revision(2)));
    let (blocked, refresh) = latest.on_event(LiveMcpEvent::RefreshFailed {
        requested: revision(1),
    });
    assert_eq!(
        (blocked.clone(), refresh),
        (
            LiveMcpState::Blocked {
                desired: revision(2)
            },
            false
        )
    );
    assert_eq!(
        blocked.on_event(LiveMcpEvent::RefreshSucceeded(revision(1))),
        (blocked.clone(), false)
    );
    assert_eq!(
        blocked.prompt_admission(&revision(2)),
        LiveMcpPromptAdmission::RefreshFirst {
            desired: revision(2)
        }
    );
}

/// Empty explicit selections also have ordering; absence of tools is not absence of a target.
#[test]
fn empty_selection_still_converges_each_generation() {
    let empty = |generation| SessionMcpRevision::for_generation(generation, vec![]);
    let (pending, refresh) =
        LiveMcpState::Active(empty(0)).on_event(LiveMcpEvent::DesiredObserved(empty(1)));
    assert!(refresh);
    assert_eq!(
        pending.prompt_admission(&empty(1)),
        LiveMcpPromptAdmission::RefreshFirst { desired: empty(1) }
    );
}
