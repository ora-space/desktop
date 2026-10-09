//! Real SQLite coverage of Revision delivery admission, the frozen plan, replay and upgrade.
use super::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;

const BASE: &str = "0123456789abcdef0123456789abcdef01234567";
const FINAL: &str = "89abcdef0123456789abcdef0123456789abcdef";
const DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Supplies a valid delivery of the session fixture's execution.
fn deliver() -> DeliverRevisionMessage {
    DeliverRevisionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("deliver-op"),
        execution_id: ExecutionId::new("deliver-execution"),
        payload: DeliverRevision {
            spec: DeliverRevisionSpec {
                node_id: NodeId::new("node"),
                session_execution_id: super::session::command().execution_id,
                checkout_execution_id: ExecutionId::new("clone"),
                base_commit: CommitId::new(BASE),
                revision_ref: RevisionRef::new("refs/ora/revisions/run-1"),
                bundle_key: ObjectKey::new("runs/1/revision.bundle"),
                history_key: ObjectKey::new("runs/1/history.jsonl"),
            },
        },
    }
}

/// The incarnation that froze the plan.
fn node() -> NodeRuntimeIdentity {
    NodeRuntimeIdentity {
        node_id: NodeId::new("node"),
        incarnation_id: NodeIncarnationId::new("first"),
    }
}

/// A delivered plan matching [`deliver`].
fn plan() -> DeliveryPlan {
    let spec = deliver().payload.spec;
    DeliveryPlan {
        directory: "frozen".into(),
        outcome: FrozenOutcome::Delivered(RevisionDelivered {
            node: node(),
            final_commit: CommitId::new(FINAL),
            base_commit: spec.base_commit,
            revision_ref: spec.revision_ref,
            bundle: StoredObject {
                key: spec.bundle_key,
                size: 10,
                sha256: Sha256Digest::new(DIGEST),
            },
            history: StoredObject {
                key: spec.history_key,
                size: 0,
                sha256: Sha256Digest::new(DIGEST),
            },
        }),
    }
}

/// Exact ACK of the single terminal event.
fn ack(sequence: u64) -> EventAckMessage {
    EventAckMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: deliver().operation_id,
        execution_id: deliver().execution_id,
        sequence: Sequence::new(sequence),
        payload: EventAck {
            node_id: NodeId::new("node"),
        },
    }
}

/// Opens a Node database bound to one Controller.
fn open(path: &std::path::Path) -> NodeDatabase {
    let mut db = NodeDatabase::open(path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    db.bind_controller(&ControllerId::new("owner")).unwrap();
    db
}

/// Admission is idempotent and starts running; the plan survives reopen until the terminal event,
/// which replays to the owner, answers status queries and is released only by its exact ACK.
#[test]
fn delivery_freezes_completes_replays_and_acknowledges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = open(&path);
    let owner = ControllerId::new("owner");
    let input = deliver();
    let accepted = db.accept_delivery(&input).unwrap();
    assert_eq!(
        accepted,
        RevisionDelivery {
            command: input.clone(),
            progress: DeliveryProgress::Preparing,
        }
    );
    assert_eq!(db.accept_delivery(&input).unwrap(), accepted);
    let mut changed = input.clone();
    changed.payload.spec.revision_ref = RevisionRef::new("refs/ora/revisions/other");
    assert!(matches!(
        db.accept_delivery(&changed),
        Err(Error::IdentityConflict)
    ));
    assert_eq!(
        db.execution_state(&input.operation_id, &input.execution_id)
            .unwrap(),
        ExecutionState::Running
    );
    assert_eq!(
        db.unfinished_runtime_executions().unwrap(),
        vec![input.execution_id.as_str().to_owned()]
    );
    db.freeze_delivery(&input.execution_id, &plan()).unwrap();
    assert!(matches!(
        db.freeze_delivery(&input.execution_id, &plan()),
        Err(Error::InvalidTransition)
    ));
    drop(db);
    let mut db = open(&path);
    assert_eq!(
        db.recoverable_deliveries().unwrap(),
        vec![RevisionDelivery {
            command: input.clone(),
            progress: DeliveryProgress::Frozen(plan()),
        }]
    );
    assert_eq!(db.controller_events(&owner).unwrap(), vec![]);
    let result = plan().outcome.result();
    db.complete_delivery(&input.execution_id, result.clone())
        .unwrap();
    assert!(matches!(
        db.complete_delivery(&input.execution_id, result.clone()),
        Err(Error::InvalidTransition)
    ));
    let event = NodeToControllerMessage::RevisionResult(RevisionResultMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: input.operation_id.clone(),
        execution_id: input.execution_id.clone(),
        sequence: Sequence::new(/*value*/ 1),
        payload: result.clone(),
    });
    assert_eq!(db.pending_events().unwrap(), vec![event.clone()]);
    assert_eq!(db.controller_events(&owner).unwrap(), vec![event.clone()]);
    assert_eq!(
        db.controller_events_after(&owner, &[]).unwrap(),
        vec![event]
    );
    assert_eq!(db.recoverable_deliveries().unwrap(), vec![]);
    assert!(db.unfinished_runtime_executions().unwrap().is_empty());
    assert!(matches!(
        db.acknowledge(&ack(/*sequence*/ 2)),
        Err(Error::InvalidAck)
    ));
    db.acknowledge(&ack(/*sequence*/ 1)).unwrap();
    db.acknowledge(&ack(/*sequence*/ 1)).unwrap();
    assert_eq!(db.pending_events().unwrap(), vec![]);
    let completed = ExecutionState::Completed(ExecutionResult::Revision(Box::new(result)));
    assert_eq!(
        db.execution_state(&input.operation_id, &input.execution_id)
            .unwrap(),
        completed
    );
}

/// A plan naming another key, ref or base than the input can never become the declaration.
#[test]
fn freezing_requires_the_inputs_keys_ref_and_base() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = open(&dir.path().join("db"));
    let input = deliver();
    db.accept_delivery(&input).unwrap();
    let mut other_key = plan();
    let FrozenOutcome::Delivered(result) = &mut other_key.outcome else {
        unreachable!("the fixture is delivered")
    };
    result.bundle.key = ObjectKey::new("runs/1/elsewhere");
    let mut escaping = plan();
    escaping.directory = "../frozen".into();
    for invalid in [other_key, escaping] {
        assert!(matches!(
            db.freeze_delivery(&input.execution_id, &invalid),
            Err(Error::IdentityConflict)
        ));
    }
    assert_eq!(
        db.recoverable_deliveries().unwrap()[0].progress,
        DeliveryProgress::Preparing
    );
    // A failure may end a delivery that never froze anything.
    db.complete_delivery(
        &input.execution_id,
        RevisionExecutionResult::RevisionFailed(RevisionFailed {
            node: node(),
            failure: RevisionFailureCode::SessionNotSettled,
        }),
    )
    .unwrap();
    assert_eq!(db.recoverable_deliveries().unwrap(), vec![]);
}

/// Session recovery ignores deliveries, and a delivery resolves its session and checkout only
/// from the matching execution families.
#[test]
fn sessions_and_deliveries_stay_separate() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = open(&dir.path().join("db"));
    let session = super::session::command();
    db.accept_session(&session).unwrap();
    db.accept_delivery(&deliver()).unwrap();
    assert_eq!(
        db.recoverable_sessions()
            .unwrap()
            .into_iter()
            .map(|record| record.command)
            .collect::<Vec<_>>(),
        vec![session.clone()]
    );
    assert_eq!(
        db.delivery_session(&session.execution_id)
            .unwrap()
            .map(|record| record.command),
        Some(session.clone())
    );
    assert_eq!(db.delivery_session(&deliver().execution_id).unwrap(), None);
    assert_eq!(db.delivery_checkout(&session.execution_id).unwrap(), None);
    assert!(matches!(
        db.find_session(&deliver().operation_id, &deliver().execution_id),
        Err(Error::IdentityConflict)
    ));
    db.session_journal()
        .unwrap()
        .end_session(&session.execution_id, super::session::ended())
        .unwrap();
    assert_eq!(db.recoverable_sessions().unwrap(), vec![]);
    assert_eq!(db.recoverable_deliveries().unwrap().len(), 1);
}

/// A controlled home refuses bare input; a controlled delivery records its permit as started.
#[test]
fn controlled_delivery_requires_a_live_permit() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = open(&dir.path().join("db"));
    let mut binding = super::repository::runtime_scope(&NodeId::new("node"));
    db.bind_runtime(&binding).unwrap();
    let input = deliver();
    assert!(matches!(
        db.accept_delivery(&input),
        Err(Error::InvalidTransition)
    ));
    binding.execution_id = input.execution_id.as_str().into();
    binding.node_operation_id = input.operation_id.as_str().into();
    let envelope = ControlledDeliverRevision {
        binding,
        command: input.clone(),
    };
    assert_eq!(
        db.accept_controlled_delivery(&envelope).unwrap().progress,
        DeliveryProgress::Preparing
    );
    // Started at admission: a later incarnation resumes it rather than re-qualifying it.
    assert!(
        db.start_controlled_execution_for(&input.execution_id, /*incarnation*/ None)
            .unwrap()
    );
}

/// Upgrading v8 keeps sessions and their attribution and admits deliveries afterwards.
#[test]
fn migrates_v8_without_losing_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = open(&path);
    let owner = ControllerId::new("owner");
    let session = super::session::command();
    db.accept_session(&session).unwrap();
    db.session_journal()
        .unwrap()
        .end_session(&session.execution_id, super::session::ended())
        .unwrap();
    let events = db.controller_events(&owner).unwrap();
    drop(db);
    super::remove_revision_schema(&path);
    let version: i64 = Connection::open(&path)
        .unwrap()
        .pragma_query_value(/*schema_name*/ None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 8);
    let mut db = open(&path);
    assert_eq!(db.controller_events(&owner).unwrap(), events);
    db.accept_delivery(&deliver()).unwrap();
    assert_eq!(db.controller_events(&owner).unwrap(), events);
    drop(db);
    let version: i64 = Connection::open(&path)
        .unwrap()
        .pragma_query_value(/*schema_name*/ None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 9);
}
