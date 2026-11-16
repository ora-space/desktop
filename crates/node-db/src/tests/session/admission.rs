//! Shared-checkout admission and its durable per-session refusal boundary.
use super::restore::reopen_after_v10_downgrade;
use super::*;
use pretty_assertions::assert_eq;

/// Gives each execution a distinct identity while retaining the same clone checkout.
fn named(id: &str) -> StartAgentSessionMessage {
    let mut input = command();
    input.operation_id = OperationId::new(format!("operation-{id}"));
    input.execution_id = ExecutionId::new(format!("execution-{id}"));
    input.payload.spec.initial_turn.turn_id = TurnId::new(format!("turn-{id}"));
    input
}

/// Gives a session a distinct identity and checkout, so per-checkout rules can be told apart
/// from per-Node ones.
fn on_checkout(id: &str, checkout: &str) -> StartAgentSessionMessage {
    let mut input = named(id);
    input.payload.spec.checkout_execution_id = ExecutionId::new(checkout);
    input
}

/// Adds valid restore input without requiring a Git fixture for admission-only cases.
fn resumed(id: &str) -> StartAgentSessionMessage {
    let mut input = named(id);
    input.payload.spec.prior_revision = Some(PriorRevision {
        revision_id: RevisionId::new(format!("revision-{id}")),
        final_commit: CommitId::new("0123456789012345678901234567890123456789"),
        git_repository_url: GitRepositoryUrl::parse("https://git.ora.test/tenant/project.git")
            .unwrap(),
        delivery_ref: DeliveryRef::parse("refs/ora/deliveries/prior").unwrap(),
        base_commit: CommitId::new("0123456789012345678901234567890123456780"),
    });
    input
}

/// Persists the service-side terminal that follows a typed storage refusal.
fn refusal_identity() -> NodeRuntimeIdentity {
    NodeRuntimeIdentity {
        node_id: NodeId::new("node"),
        incarnation_id: NodeIncarnationId::new("incarnation"),
    }
}

/// A refused session's durable terminal, shared by the whole-record assertions below.
fn refused_record(command: &StartAgentSessionMessage, detail: &str) -> SessionExecution {
    SessionExecution {
        command: command.clone(),
        state: ExecutionState::Completed(ExecutionResult::AgentSession(
            AgentSessionResult::AgentSessionEnded(AgentSessionEnded {
                node: refusal_identity(),
                reason: AgentSessionEndReason::AgentFailed,
                detail: Some(detail.into()),
                plugin_id: None,
                setting_id: None,
                transport: None,
                scheduled_result: None,
                artifact_manifest: None,
            }),
        )),
        last_sequence: 1,
    }
}

/// Admits a start the checkout must refuse, returning what admission hands the service.
fn refused<G: WriteGuard>(
    db: &mut NodeDatabase<G>,
    input: &StartAgentSessionMessage,
) -> RefusedSession {
    match db.accept_session(input) {
        Err(Error::SessionRefused(refused)) => refused,
        other => panic!("start was not refused: {other:?}"),
    }
}

/// A live runtime permit for exactly one controlled session input.
fn permit_for(db: &NodeDatabase, command: &StartAgentSessionMessage) -> RuntimeBinding {
    let mut permit = super::super::repository::runtime_scope(db.node_id());
    permit.execution_id = command.execution_id.as_str().into();
    permit.node_operation_id = command.operation_id.as_str().into();
    permit
}

/// The refused identity, terminal and event roll back together when the outbox write fails.
#[test]
fn refusal_terminal_and_outbox_are_atomic() {
    let directory = tempfile::tempdir().unwrap();
    let fault = std::rc::Rc::new(std::cell::Cell::new(/*value*/ None));
    let mut db = NodeDatabase::open_with_guard(
        &directory.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
        Fault(fault.clone()),
    )
    .unwrap();
    for index in 0..MAX_SESSIONS_PER_CHECKOUT {
        db.accept_session(&named(&index.to_string())).unwrap();
    }
    let ninth = named("ninth");
    assert_eq!(
        refusal_of(&db.accept_session(&ninth)),
        Some(SessionRefusal::CheckoutSessionLimit)
    );
    let refusal = refused(&mut db, &ninth);
    fault.set(Some(WritePoint::Outbox));
    assert!(matches!(
        db.refuse_session(&refusal, &refusal_identity()),
        Err(Error::Injected(WritePoint::Outbox))
    ));
    assert_eq!(
        db.find_session(&ninth.operation_id, &ninth.execution_id)
            .unwrap(),
        None
    );
    assert!(db.pending_events().unwrap().is_empty());
    fault.set(/*val*/ None);
    assert_eq!(
        db.refuse_session(&refusal, &refusal_identity())
            .unwrap()
            .last_sequence,
        1
    );
}

/// Ordinary sessions share one checkout up to eight, and a terminal frees exactly one slot.
#[test]
fn plain_sessions_share_a_checkout_until_the_named_limit() {
    let directory = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &directory.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    for id in ["first", "second"] {
        assert_eq!(
            db.accept_session(&named(id)).unwrap().state,
            ExecutionState::Accepted
        );
    }
    for index in 2..MAX_SESSIONS_PER_CHECKOUT {
        db.accept_session(&named(&index.to_string())).unwrap();
    }
    let ninth = named("ninth");
    assert_eq!(
        refusal_of(&db.accept_session(&ninth)),
        Some(SessionRefusal::CheckoutSessionLimit)
    );
    let refusal = refused(&mut db, &ninth);
    let record = db.refuse_session(&refusal, &refusal_identity()).unwrap();
    assert_eq!(record, refused_record(&ninth, "checkout_session_limit"));
    assert_eq!(
        db.refuse_session(&refusal, &refusal_identity()).unwrap(),
        record
    );
    // A retransmitted start finds the durable refusal before admission runs again.
    assert_eq!(db.accept_session(&ninth).unwrap(), record);
    assert_eq!(db.pending_events().unwrap().len(), 1);

    let journal = db.session_journal().unwrap();
    journal
        .end_session(&named("first").execution_id, ended())
        .unwrap();
    assert_eq!(
        db.accept_session(&named("after-slot")).unwrap().state,
        ExecutionState::Accepted
    );
}

/// The limit counts unfinished sessions per checkout, not per Node: a full checkout never
/// refuses a session another clone owns.
#[test]
fn limit_counts_each_checkout_separately() {
    let directory = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &directory.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    for index in 0..MAX_SESSIONS_PER_CHECKOUT {
        db.accept_session(&named(&index.to_string())).unwrap();
    }
    assert_eq!(
        refusal_of(&db.accept_session(&named("ninth"))),
        Some(SessionRefusal::CheckoutSessionLimit)
    );
    assert_eq!(
        db.accept_session(&on_checkout("ninth", "other-clone"))
            .unwrap()
            .state,
        ExecutionState::Accepted
    );
}

/// A checkout at its limit still reports why a prior-revision start is impossible: restore
/// needs the checkout alone, so the busy class is named before the count is consulted.
#[test]
fn a_prior_revision_start_at_the_limit_reports_busy() {
    let directory = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &directory.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    for index in 0..MAX_SESSIONS_PER_CHECKOUT {
        db.accept_session(&named(&index.to_string())).unwrap();
    }
    assert_eq!(
        refusal_of(&db.accept_session(&resumed("restore"))),
        Some(SessionRefusal::CheckoutBusy)
    );
}

/// A new restore never moves a checkout out from under an ordinary live sibling.
#[test]
fn prior_revision_is_refused_while_a_sibling_is_unfinished() {
    let directory = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &directory.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    db.accept_session(&named("plain")).unwrap();
    let restore = resumed("restore");
    assert_eq!(
        refusal_of(&db.accept_session(&restore)),
        Some(SessionRefusal::CheckoutBusy)
    );
    let refusal = refused(&mut db, &restore);
    assert_eq!(
        db.refuse_session(&refusal, &refusal_identity()).unwrap(),
        refused_record(&restore, "checkout_busy")
    );
    assert_eq!(
        db.session_journal()
            .unwrap()
            .restore_phase(&restore.execution_id)
            .unwrap(),
        Some(RestorePhase::Pending)
    );
}

/// A plain Agent cannot start while its sibling is still restoring the shared tree.
#[test]
fn plain_session_is_refused_while_a_sibling_restore_is_pending() {
    let directory = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &directory.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    db.accept_session(&resumed("restore")).unwrap();
    assert_eq!(
        refusal_of(&db.accept_session(&named("plain"))),
        Some(SessionRefusal::CheckoutBusy)
    );
}

/// A plain session waits until an unfinished sibling restore reaches Agent admission: between
/// `restored` and `agent_admitted` the sibling still prepares the checkout, so sharing starts
/// only once its Agent is admitted.
#[test]
fn plain_session_waits_for_the_sibling_restores_agent_admission() {
    let directory = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &directory.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let restore = resumed("restore");
    db.accept_session(&restore).unwrap();
    db.start_session(&restore, &NodeIncarnationId::new("incarnation"))
        .unwrap();
    let journal = db.session_journal().unwrap();
    let process = ProcessAttempt {
        execution: restore.execution_id.clone(),
        id: AttemptId::generate(),
    };
    journal
        .record_restore_attempt(&process, &git_command(), StopContainment::BestEffort)
        .unwrap();
    journal.confirm_restore_stop(&process.id).unwrap();
    journal.restored(&restore.execution_id).unwrap();

    assert_eq!(
        refusal_of(&db.accept_session(&named("plain"))),
        Some(SessionRefusal::CheckoutBusy)
    );
    journal.agent_admitted(&restore.execution_id).unwrap();
    assert_eq!(
        db.accept_session(&named("plain")).unwrap().state,
        ExecutionState::Accepted
    );
    // The restore is unfinished either way, so another restore still needs the checkout alone.
    assert_eq!(
        refusal_of(&db.accept_session(&resumed("another-restore"))),
        Some(SessionRefusal::CheckoutBusy)
    );
}

/// A plain session cannot start on a checkout whose pre-journal restore still has unknown stop
/// evidence, whatever the business state of that restore's execution.
#[test]
fn legacy_unknown_fences_a_plain_session() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node")))?;
    // An in-flight restore whose stop evidence predates the journal survives the upgrade as
    // legacy_unknown (see the restore suites for the full responsibility contract).
    let input = resumed("legacy");
    db.accept_session(&input)?;
    assert!(db.start_session(&input, &NodeIncarnationId::new("original"))?);
    drop(db);
    let mut db = reopen_after_v10_downgrade(&path)?;
    assert_eq!(
        db.session_journal()?.restore_phase(&input.execution_id)?,
        Some(RestorePhase::LegacyUnknown)
    );
    // The replacement carries no prior_revision, so only the unknown-stop fence can refuse it.
    assert_eq!(
        refusal_of(&db.accept_session(&named("plain"))),
        Some(SessionRefusal::CheckoutBusy)
    );
    Ok(())
}

/// A controlled start never reaches checkout classification: the runtime singleton refuses any
/// unfinished execution first, keeping the connection-closing contract it always had.
#[test]
fn controlled_start_keeps_the_runtime_singleton_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &directory.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let scope = super::super::repository::runtime_scope(db.node_id());
    db.bind_runtime(&scope).unwrap();
    // A first controlled session with a pending restore makes the checkout busy for any other
    // start, so a swapped order would classify the second start instead of refusing the slot.
    let restore = resumed("restore");
    db.accept_controlled_session(&restore, &permit_for(&db, &restore))
        .unwrap();
    let second = named("second");
    assert!(matches!(
        db.accept_controlled_session(&second, &permit_for(&db, &second)),
        Err(Error::ResourceConflict)
    ));
}
