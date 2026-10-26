use super::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;

/// Builds a proof request for the very same real delivery used by the acceptance tests.
fn request<G: WriteGuard>(db: &NodeDatabase<G>) -> (ExecutionFence, ControlledDeliverRevision) {
    let command = super::revision::deliver();
    let mut binding = super::repository::runtime_scope(db.node_id());
    binding.execution_id = command.execution_id.as_str().to_owned();
    binding.node_operation_id = command.operation_id.as_str().to_owned();
    let fence = ExecutionFence {
        authority: binding.clone(),
        origin: ExecutionOrigin {
            binding: binding.clone(),
            work_id: "original-work".to_owned(),
            input_json: r#"{"kind":"deliver_revision"}"#.to_owned(),
        },
    };
    (fence, ControlledDeliverRevision { binding, command })
}

/// Disk failures cannot emit a success proof or leave a half-committed input fence.
#[test]
fn a_failed_fence_write_leaves_no_proof_and_does_not_consume_intake() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let (fence, delivery) = request(&db);
    db.bind_runtime(&fence.authority).unwrap();
    db.connection.execute_batch("CREATE TRIGGER fail_fence BEFORE INSERT ON execution_non_acceptance BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(db.fence_unaccepted_execution(&fence).is_err());
    let count: i64 = db
        .connection
        .query_row("SELECT count(*) FROM execution_non_acceptance", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    db.connection
        .execute_batch("DROP TRIGGER fail_fence")
        .unwrap();
    db.accept_controlled_delivery(&delivery).unwrap();
    assert_eq!(db.fence_unaccepted_execution(&fence).unwrap(), None);
}

/// Upgrade adds fences without rewriting prior admission or inventing proof for existing work.
#[test]
fn upgrading_v12_preserves_accepted_execution_and_adds_v13_fences() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let (fence, delivery) = request(&db);
    db.bind_runtime(&fence.authority).unwrap();
    let accepted = db.accept_controlled_delivery(&delivery).unwrap();
    db.connection.execute_batch("DROP TRIGGER refuse_fenced_execution; DROP TABLE execution_non_acceptance; PRAGMA user_version=12;").unwrap();
    drop(db);
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let version: i64 = db
        .connection
        .pragma_query_value(/*schema_name*/ None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 13);
    assert_eq!(
        db.find_delivery(
            &delivery.command.operation_id,
            &delivery.command.execution_id
        )
        .unwrap(),
        Some(accepted)
    );
    assert_eq!(db.fence_unaccepted_execution(&fence).unwrap(), None);
}

/// Proof and admission race on the same SQLite writer: the proof wins, so every later intake fails.
#[test]
fn a_durable_fence_refuses_late_intake_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let (fence, delivery) = request(&db);
    db.bind_runtime(&fence.authority).unwrap();
    let proof = db.fence_unaccepted_execution(&fence).unwrap().unwrap();
    assert_eq!(
        db.fence_unaccepted_execution(&fence).unwrap(),
        Some(proof.clone())
    );
    assert!(db.accept_controlled_delivery(&delivery).is_err());
    assert!(
        db.find_delivery(
            &delivery.command.operation_id,
            &delivery.command.execution_id
        )
        .unwrap()
        .is_none()
    );
    drop(db);
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let mut next = fence;
    next.authority.node_instance_id = "next-instance".to_owned();
    next.authority.node_incarnation_id = "next-incarnation".to_owned();
    next.authority.control_epoch += 1;
    db.bind_runtime(&next.authority).unwrap();
    assert_eq!(db.fence_unaccepted_execution(&next).unwrap(), Some(proof));
    let mut late = delivery;
    late.binding = next.authority;
    assert!(db.accept_controlled_delivery(&late).is_err());
}

/// Accepted work and even completed history outrank a later administrative request for absence.
#[test]
fn accepted_execution_cannot_be_reinterpreted_as_unaccepted() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let (fence, delivery) = request(&db);
    db.bind_runtime(&fence.authority).unwrap();
    let accepted = db.accept_controlled_delivery(&delivery).unwrap();
    assert_eq!(db.fence_unaccepted_execution(&fence).unwrap(), None);
    assert_eq!(
        db.find_delivery(
            &delivery.command.operation_id,
            &delivery.command.execution_id
        )
        .unwrap(),
        Some(accepted)
    );
}

/// An empty replacement ledger is not evidence about the dispatch's original Node incarnation.
#[test]
fn a_new_incarnation_cannot_create_proof_for_an_old_empty_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let (mut fence, _) = request(&db);
    fence.authority.node_incarnation_id = "replacement".to_owned();
    fence.authority.node_instance_id = "replacement-instance".to_owned();
    db.bind_runtime(&fence.authority).unwrap();
    assert_eq!(db.fence_unaccepted_execution(&fence).unwrap(), None);
}
