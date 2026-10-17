//! Admission and compatibility boundaries for the durable session ledger.
use super::*;
use pretty_assertions::assert_eq;

/// Both keys are immutable across execution families and duplicate input must match in full.
#[test]
fn admission_rejects_identity_rebinding_and_wrong_node() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let original = command();
    let accepted = db.accept_session(&original).unwrap();
    let mut changed = original.clone();
    changed.payload.spec.agent_plugin_version = PluginVersion::new("2.0.0");
    assert!(matches!(
        db.accept_session(&changed),
        Err(Error::IdentityConflict)
    ));
    changed = original.clone();
    changed.operation_id = OperationId::new("other");
    assert!(matches!(
        db.accept_session(&changed),
        Err(Error::IdentityConflict)
    ));
    changed = original.clone();
    changed.payload.spec.node_id = NodeId::new("other");
    assert!(matches!(
        db.accept_session(&changed),
        Err(Error::NodeMismatch)
    ));
    let PluginCommand::Remove(mut plugin) = super::super::plugin::command() else {
        unreachable!()
    };
    plugin.operation_id = original.operation_id.clone();
    plugin.execution_id = original.execution_id.clone();
    assert!(matches!(
        db.accept_plugins(&PluginCommand::Remove(plugin)),
        Err(Error::IdentityConflict)
    ));
    assert_eq!(db.recoverable_sessions().unwrap(), vec![accepted]);
}

/// Admission requires an open scope; start rechecks incarnation, closure and outstanding liability.
#[test]
fn controlled_sessions_respect_closure_and_incarnation() {
    for close in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
        let mut scope = super::super::repository::runtime_scope(db.node_id());
        db.bind_runtime(&scope).unwrap();
        let input = command();
        let mut permit = scope.clone();
        permit.execution_id = input.execution_id.as_str().into();
        permit.node_operation_id = input.operation_id.as_str().into();
        assert!(db.accept_session(&input).is_err());
        db.accept_controlled_session(&input, &permit).unwrap();
        assert_eq!(
            db.unfinished_runtime_executions().unwrap(),
            vec![input.execution_id.as_str()]
        );
        let mut other = input.clone();
        other.operation_id = OperationId::new("other-op");
        other.execution_id = ExecutionId::new("other-exec");
        let mut other_permit = permit.clone();
        other_permit.execution_id = other.execution_id.as_str().into();
        other_permit.node_operation_id = other.operation_id.as_str().into();
        assert!(matches!(
            db.accept_controlled_session(&other, &other_permit),
            Err(Error::ResourceConflict)
        ));
        drop(db);
        let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
        if close {
            scope.input_closed = true;
            scope.control_version += 1;
            db.bind_runtime(&scope).unwrap();
        } else {
            db.enforce_runtime_incarnation("replacement").unwrap();
        }
        assert!(
            !db.start_session(&input, &NodeIncarnationId::new("replacement"))
                .unwrap()
        );
        assert!(db.accept_controlled_session(&input, &permit).is_err());
        db.session_journal()
            .unwrap()
            .end_session(&input.execution_id, ended())
            .unwrap();
        assert!(db.unfinished_runtime_executions().unwrap().is_empty());
    }
}

/// Upgrading v7 retains plugin identity, terminal evidence and Controller attribution.
#[test]
fn migrates_v7_without_losing_existing_executions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let owner = ControllerId::new("owner");
    db.bind_controller(&owner).unwrap();
    let plugin = super::super::plugin::command();
    db.accept_plugins(&plugin).unwrap();
    db.complete_plugins(&plugin, super::super::plugin::result())
        .unwrap();
    let events = db.controller_events(&owner).unwrap();
    drop(db);
    super::super::remove_session_schema(&path);
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(db.controller_events(&owner).unwrap(), events);
    assert_eq!(
        db.find_plugins(plugin.operation_id(), plugin.execution_id())
            .unwrap()
            .unwrap()
            .state,
        ExecutionState::Completed(ExecutionResult::Plugin(super::super::plugin::result()))
    );
    db.accept_session(&command()).unwrap();
    db.session_journal()
        .unwrap()
        .end_session(&command().execution_id, ended())
        .unwrap();
    assert_eq!(db.controller_events(&owner).unwrap().len(), 2);
}

/// Checkout resolution requires successful clone evidence and survives reopening without path guessing.
#[test]
fn checkout_requires_a_successful_clone() {
    use super::super::repository::{clone_fixture, clone_ready, dispatch};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let journal = db.session_journal().unwrap();
    assert_eq!(
        journal.checkout(&ExecutionId::new("missing")).unwrap(),
        None
    );
    db.accept_session(&command()).unwrap();
    assert_eq!(journal.checkout(&command().execution_id).unwrap(), None);
    let (input, target) = clone_fixture(db.node_id(), dir.path());
    let accepted = db.accept_clone(&input, &target).unwrap();
    assert_eq!(journal.checkout(&input.execution_id).unwrap(), None);
    let (record, attempt) = dispatch(&mut db, &accepted);
    let processes = db.process_journal().unwrap();
    processes
        .record_outcome(attempt.intent.run, /*exit_code*/ 0)
        .unwrap();
    processes.cleaned(attempt.intent.run).unwrap();
    db.complete_clone(&record, clone_ready(&record)).unwrap();
    assert_eq!(
        journal.checkout(&input.execution_id).unwrap(),
        Some(target.path.clone())
    );
    drop(processes);
    drop(journal);
    drop(db);
    let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(
        db.session_journal()
            .unwrap()
            .checkout(&input.execution_id)
            .unwrap(),
        Some(target.path)
    );
}

/// A racing admission either commits before the end and is discarded, or is rejected without a row.
#[test]
fn terminal_write_serializes_with_command_admission() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    db.accept_session(&command()).unwrap();
    let journal = db.session_journal().unwrap();
    let observer = journal.clone();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(/*n*/ 2));
    let actor_barrier = barrier.clone();
    let actor = std::thread::spawn(move || {
        actor_barrier.wait();
        journal
            .end_session(&command().execution_id, ended())
            .unwrap();
    });
    barrier.wait();
    let admission = db.accept_session_command(&turn("race")).unwrap();
    actor.join().unwrap();
    let expected = match admission {
        CommandAdmission::Accepted => Some(SessionCommandState::Discarded),
        CommandAdmission::SessionEnded => None,
    };
    assert_eq!(
        observer
            .command_state(&command().execution_id, &CommandId::new("race"))
            .unwrap(),
        expected
    );
    assert!(
        observer
            .queued_commands(&command().execution_id)
            .unwrap()
            .is_empty()
    );
}

/// Unknown and failed clones cannot supply a checkout even if their destination was retained.
#[test]
fn checkout_rejects_unknown_and_failed_clone() {
    use super::super::repository::{clone_fixture, dispatch};
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(&dir.path().join("db"), NodeIdentity::Discover).unwrap();
    let (input, target) = clone_fixture(db.node_id(), dir.path());
    let accepted = db.accept_clone(&input, &target).unwrap();
    let (record, attempt) = dispatch(&mut db, &accepted);
    let unknown = db
        .advance_clone(
            &record,
            CloneProgress::Unknown(ClonePhase::Dispatched {
                identity: "native-directory-identity".into(),
                step: 0,
            }),
        )
        .unwrap();
    let journal = db.session_journal().unwrap();
    assert_eq!(journal.checkout(&input.execution_id).unwrap(), None);
    let processes = db.process_journal().unwrap();
    processes
        .record_outcome(attempt.intent.run, /*exit_code*/ 128)
        .unwrap();
    processes.cleaned(attempt.intent.run).unwrap();
    db.complete_clone(
        &unknown,
        CloneExecutionResult::CloneFailed(CloneFailed {
            node: NodeRuntimeIdentity {
                node_id: db.node_id().clone(),
                incarnation_id: NodeIncarnationId::new("first"),
            },
            spec: input.payload.spec,
            failure: CloneFailureCode::SourceUnavailable,
            residual: CloneResidual::Retained {
                repository_id: target.repository_id,
                path: NodePath::new(target.path.to_str().unwrap()),
            },
        }),
    )
    .unwrap();
    assert_eq!(journal.checkout(&input.execution_id).unwrap(), None);
}

/// Session commands require the refreshed original scope, never an expired dispatch permit or a new epoch.
#[test]
fn session_command_authority_expires_and_closes_without_losing_existing_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let mut scope = super::super::repository::runtime_scope(db.node_id());
    db.bind_runtime(&scope).unwrap();
    let mut permit = scope.clone();
    permit.execution_id = command().execution_id.as_str().into();
    permit.node_operation_id = command().operation_id.as_str().into();
    db.accept_controlled_session(&command(), &permit).unwrap();
    let incarnation = NodeIncarnationId::new("host-incarnation");
    db.start_session(&command(), &incarnation).unwrap();
    db.authorize_session_command(&command().execution_id, &incarnation)
        .unwrap();
    assert!(
        db.authorize_session_command(&command().execution_id, &NodeIncarnationId::new("other"))
            .is_err()
    );
    // Model durable time passing without sleeping or mutating the process clock.
    let mut expired = scope.clone();
    expired.issued_at_ms -= 60_000;
    expired.expires_at_ms -= 60_000;
    db.connection
        .execute(
            "UPDATE runtime_binding SET data=?1",
            [serde_json::to_string(&expired).unwrap()],
        )
        .unwrap();
    assert!(
        db.authorize_session_command(&command().execution_id, &incarnation)
            .is_err()
    );
    scope.control_version += 1;
    db.bind_runtime(&scope).unwrap();
    db.authorize_session_command(&command().execution_id, &incarnation)
        .unwrap();
    scope.input_closed = true;
    scope.control_version += 1;
    db.bind_runtime(&scope).unwrap();
    assert!(
        db.authorize_session_command(&command().execution_id, &incarnation)
            .is_err()
    );
    assert_eq!(
        db.execution_state(&command().operation_id, &command().execution_id)
            .unwrap(),
        ExecutionState::Running
    );
}

/// A full stream cannot hide another execution's events, and cursors never consume durable rows.
#[test]
fn bounded_replay_skips_full_streams_without_acknowledging_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(
        &dir.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )
    .unwrap();
    let owner = ControllerId::new("owner");
    db.bind_controller(&owner).unwrap();
    let mut first = command();
    first.execution_id = ExecutionId::new("a");
    db.accept_session(&first).unwrap();
    db.start_session(&first, &NodeIncarnationId::new("first"))
        .unwrap();
    let journal = db.session_journal().unwrap();
    for _ in 0..20 {
        journal
            .append_thread_event(&first.execution_id, event())
            .unwrap();
    }
    let mut second = command();
    second.execution_id = ExecutionId::new("b");
    second.operation_id = OperationId::new("b-op");
    db.accept_session(&second).unwrap();
    journal.end_session(&second.execution_id, ended()).unwrap();
    let before = db.pending_events().unwrap();
    assert_eq!(
        db.controller_events_after(&owner, &[]).unwrap(),
        before[..16]
    );
    assert_eq!(
        db.controller_events_after(
            &owner,
            &[crate::EventCursor {
                execution: first.execution_id.clone(),
                after: 16,
                remaining: 0
            }]
        )
        .unwrap(),
        before[20..]
    );
    assert_eq!(
        db.controller_events_after(
            &owner,
            &[crate::EventCursor {
                execution: first.execution_id,
                after: 16,
                remaining: 240
            }]
        )
        .unwrap(),
        before[16..]
    );
    assert!(
        db.controller_events_after(&ControllerId::new("other"), &[])
            .unwrap()
            .is_empty()
    );
    assert_eq!(db.pending_events().unwrap(), before);
}
