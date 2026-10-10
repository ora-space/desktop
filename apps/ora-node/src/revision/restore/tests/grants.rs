//! The memory-only download grant store: what it accepts, what it asks for, and what it forgets.
use super::super::grants::{Answer, DownloadGrants};
use super::*;
use pretty_assertions::assert_eq;
use std::time::Duration;

const KEY: &str = "runs/1/revision.bundle";

/// A grant for `key` valid for `valid_for`.
fn grant(key: &str, valid_for: Duration) -> ObjectDownloadGrant {
    ObjectDownloadGrant {
        object_key: ObjectKey::new(key),
        url: PresignedUrl::new("https://store.example/read?X-Amz-Signature=secret"),
        method: DownloadMethod::Get,
        headers: [("x-amz-meta-proof".to_owned(), "signed".to_owned())].into(),
        expires_at: ora_logging::clock::now_local() + valid_for,
    }
}

/// An answer to `execution` of `operation`.
fn answer(operation: &str, execution: &str, payload: DownloadGrant) -> DownloadGrantMessage {
    DownloadGrantMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new(operation),
        execution_id: ExecutionId::new(execution),
        payload,
    }
}

/// The request a restore of `execution` sends.
fn needed(execution: &str) -> DownloadGrantNeededMessage {
    DownloadGrantNeededMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("run"),
        execution_id: ExecutionId::new(execution),
        payload: DownloadGrantNeeded {
            node_id: NodeId::new("node"),
        },
    }
}

/// Only an answer for the registered execution, operation and object counts; an expired grant is
/// dropped, and a refusal is surfaced as such.
#[test]
fn accepts_only_the_registered_restores_object() {
    ora_logging::initialize_test_clock();
    let grants = DownloadGrants::new();
    let registration = grants.register(
        OperationId::new("run"),
        ExecutionId::new("session"),
        NodeId::new("node"),
        ObjectKey::new(KEY),
    );
    let margin = Duration::from_secs(/*secs*/ 5);
    let long = Duration::from_secs(/*secs*/ 3600);
    for foreign in [
        answer("run", "other-session", DownloadGrant::Refused {}),
        answer("other-run", "session", DownloadGrant::Refused {}),
        answer(
            "run",
            "session",
            DownloadGrant::Granted {
                grants: vec![grant("runs/other/revision.bundle", long)],
            },
        ),
    ] {
        grants.offer(foreign);
        assert!(registration.answer(margin).is_none());
    }
    grants.offer(answer(
        "run",
        "session",
        DownloadGrant::Granted {
            grants: vec![grant(KEY, Duration::from_secs(/*secs*/ 1))],
        },
    ));
    assert!(
        registration.answer(margin).is_none(),
        "expires inside the margin"
    );
    let usable = grant(KEY, long);
    grants.offer(answer(
        "run",
        "session",
        DownloadGrant::Granted {
            grants: vec![usable.clone()],
        },
    ));
    let Some(Answer::Granted(held)) = registration.answer(margin) else {
        panic!("the registered object's grant is kept");
    };
    assert_eq!(held, usable);
    registration.discard();
    assert!(registration.answer(margin).is_none());
    grants.offer(answer("run", "session", DownloadGrant::Refused {}));
    assert!(matches!(registration.answer(margin), Some(Answer::Refused)));
}

/// A waiting restore's request is broadcast and repeated to every new connection until it is
/// answered; dropping the registration forgets it and any grant.
#[test]
fn waiting_requests_repeat_until_answered_and_end_with_the_registration() {
    ora_logging::initialize_test_clock();
    let grants = DownloadGrants::new();
    let mut requests = grants.subscribe();
    let registration = grants.register(
        OperationId::new("run"),
        ExecutionId::new("session"),
        NodeId::new("node"),
        ObjectKey::new(KEY),
    );
    assert_eq!(grants.pending(), vec![]);
    registration.request();
    assert_eq!(requests.try_recv().unwrap(), needed("session"));
    assert_eq!(grants.pending(), vec![needed("session")]);
    grants.offer(answer(
        "run",
        "session",
        DownloadGrant::Granted {
            grants: vec![grant(KEY, Duration::from_secs(/*secs*/ 3600))],
        },
    ));
    registration.answer(Duration::ZERO).unwrap();
    assert_eq!(grants.pending(), vec![]);
    registration.request();
    drop(registration);
    assert_eq!(grants.pending(), vec![]);
    grants.offer(answer("run", "session", DownloadGrant::Refused {}));
    let again = grants.register(
        OperationId::new("run"),
        ExecutionId::new("session"),
        NodeId::new("node"),
        ObjectKey::new(KEY),
    );
    assert!(
        again.answer(Duration::ZERO).is_none(),
        "nothing outlives a registration"
    );
}
