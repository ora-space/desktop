use super::*;
use ora_node_protocol::*;
use ora_process_protocol::*;
use pretty_assertions::assert_eq;

/// Supplies a clone without requiring a Main Workspace or performing filesystem mutations.
fn clone_fixture(node: &NodeId, root: &std::path::Path) -> (CloneRepositoryMessage, CloneTarget) {
    (
        CloneRepositoryMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(RequestId::new("request-clone")),
            operation_id: OperationId::new("operation-clone"),
            execution_id: ExecutionId::new("execution-clone"),
            payload: CloneRepository {
                spec: CloneExecutionSpec {
                    node_id: node.clone(),
                    repository: CloneRepositoryUrl::parse("https://example.com/repo.git").unwrap(),
                    branch: BranchName::new("feature/clone"),
                },
            },
        },
        CloneTarget {
            repository_id: RepositoryId::new("repository-1"),
            root: root.to_owned(),
            path: root.join("repository-1"),
        },
    )
}

/// Establishes only local dispatch intent; neither this helper nor the store starts a process.
fn dispatch<G: WriteGuard>(
    db: &mut NodeDatabase<G>,
    record: &CloneExecution,
) -> (CloneExecution, ProcessAttempt) {
    let created = db
        .advance_clone(
            record,
            CloneProgress::Pending(ClonePhase::DirectoryCreated {
                identity: "native-directory-identity".into(),
            }),
        )
        .unwrap();
    let journal = db.process_journal().unwrap();
    journal.manage_clone(&created).unwrap();
    let dispatched = db
        .advance_clone(
            &created,
            CloneProgress::Pending(ClonePhase::Dispatched {
                identity: "native-directory-identity".into(),
            }),
        )
        .unwrap();
    let attempt = ProcessAttempt {
        execution: record.command.execution_id.clone(),
        host_directory: record.target.root.join("host"),
        expected_uid: 42,
        intent: HostRunIntent {
            scope: ScopeId::new(),
            run: RunId::new(),
            spec: RunSpec::new("git", &record.target.root, DescendantPolicy::WaitForAll),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        },
    };
    journal.record(&attempt).unwrap();
    (dispatched, attempt)
}

/// Uses full observed facts and original input, not a branch tip fetched during replay.
fn clone_ready(record: &CloneExecution) -> CloneExecutionResult {
    CloneExecutionResult::CloneReady(CloneReady {
        node: NodeRuntimeIdentity {
            node_id: record.command.payload.spec.node_id.clone(),
            incarnation_id: NodeIncarnationId::new("original"),
        },
        spec: record.command.payload.spec.clone(),
        repository_id: record.target.repository_id.clone(),
        path: NodePath::new(record.target.path.to_str().unwrap()),
        commit: CommitId::new("0123456789abcdef0123456789abcdef01234567"),
    })
}

/// Acceptance freezes input and target without touching the destination, including after restart.
#[test]
fn clone_acceptance_deduplicates_and_reserves_across_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("node.db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    let (command, target) = clone_fixture(db.node_id(), dir.path());
    let record = db.accept_clone(&command, &target).unwrap();
    assert!(!target.path.exists());
    let mut changed_target = target.clone();
    changed_target.path = dir.path().join("must-not-allocate");
    assert_eq!(db.accept_clone(&command, &changed_target).unwrap(), record);
    let mut changed = command.clone();
    changed.payload.spec.branch = BranchName::new("other");
    assert!(matches!(
        db.accept_clone(&changed, &target),
        Err(Error::IdentityConflict)
    ));
    let (mut worktree, mut wt_target) = fixture(db.node_id());
    let Command::Ensure(message) = &mut worktree else {
        panic!("fixture")
    };
    message.operation_id = command.operation_id.clone();
    assert!(matches!(
        db.accept(&worktree, &wt_target),
        Err(Error::IdentityConflict)
    ));
    let Command::Ensure(message) = &mut worktree else {
        panic!("fixture")
    };
    message.operation_id = OperationId::new("independent");
    wt_target.path = target.path.join("child");
    assert!(matches!(
        db.accept(&worktree, &wt_target),
        Err(Error::ResourceConflict)
    ));
    drop(db);
    let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(
        db.find_clone(&command.operation_id, &command.execution_id)
            .unwrap(),
        Some(record.clone())
    );
    assert_eq!(db.recoverable_clones().unwrap(), vec![record]);
}

/// Success requires the original Run's observed success and cleanup, and completion/outbox are atomic.
#[test]
fn clone_completion_requires_process_evidence_and_survives_exact_ack() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("node.db");
    let fault = std::rc::Rc::new(std::cell::Cell::new(None));
    let mut db =
        NodeDatabase::open_with_guard(&path, NodeIdentity::Discover, Fault(fault.clone())).unwrap();
    let (command, target) = clone_fixture(db.node_id(), dir.path());
    let accepted = db.accept_clone(&command, &target).unwrap();
    assert!(
        db.complete_clone(&accepted, clone_ready(&accepted))
            .is_err()
    );
    let (record, attempt) = dispatch(&mut db, &accepted);
    let result = clone_ready(&record);
    let journal = db.process_journal().unwrap();
    assert!(matches!(
        db.complete_clone(&record, result.clone()),
        Err(Error::InvalidTransition)
    ));
    journal
        .record_outcome(attempt.intent.run, /*exit_code*/ 0)
        .unwrap();
    assert!(matches!(
        db.complete_clone(&record, result.clone()),
        Err(Error::InvalidTransition)
    ));
    journal.cleaned(attempt.intent.run).unwrap();
    let mut second = attempt.clone();
    second.intent.run = RunId::new();
    assert!(matches!(
        journal.record(&second),
        Err(Error::InvalidTransition)
    ));
    assert!(matches!(
        journal.record_outcome(attempt.intent.run, /*exit_code*/ 1),
        Err(Error::IdentityConflict)
    ));
    fault.set(Some(WritePoint::Outbox));
    assert!(matches!(
        db.complete_clone(&record, result.clone()),
        Err(Error::Injected(WritePoint::Outbox))
    ));
    assert_eq!(
        db.find_clone(&command.operation_id, &command.execution_id)
            .unwrap(),
        Some(record.clone())
    );
    assert_eq!(db.pending_events().unwrap(), vec![]);
    fault.set(/*val*/ None);
    db.complete_clone(&record, result.clone()).unwrap();
    let expected = vec![record.event(result.clone())];
    drop(journal);
    drop(db);
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(db.pending_events().unwrap(), expected);
    assert_eq!(
        db.execution_state(&command.operation_id, &command.execution_id)
            .unwrap(),
        ExecutionState::Completed(ExecutionResult::Clone(result.clone()))
    );
    assert_eq!(db.pending_events().unwrap(), expected);
    let mut ack = EventAckMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command.operation_id.clone(),
        execution_id: command.execution_id.clone(),
        sequence: Sequence::new(/*value*/ 2),
        payload: EventAck {
            node_id: db.node_id().clone(),
        },
    };
    assert!(matches!(db.acknowledge(&ack), Err(Error::InvalidAck)));
    ack.sequence = Sequence::new(/*value*/ 1);
    db.acknowledge(&ack).unwrap();
    db.acknowledge(&ack).unwrap();
    assert_eq!(db.pending_events().unwrap(), vec![]);
    assert_eq!(
        db.accept_clone(&command, &target).unwrap().progress,
        CloneProgress::Completed(result)
    );
}

/// Unknown and failed attempts cannot release reservations or change their directory identity.
#[test]
fn clone_failure_retains_destination_and_unknown_never_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(&dir.path().join("db"), NodeIdentity::Discover).unwrap();
    let (command, target) = clone_fixture(db.node_id(), dir.path());
    let accepted = db.accept_clone(&command, &target).unwrap();
    let (record, attempt) = dispatch(&mut db, &accepted);
    let unknown = db
        .advance_clone(
            &record,
            CloneProgress::Unknown(ClonePhase::Dispatched {
                identity: "native-directory-identity".into(),
            }),
        )
        .unwrap();
    assert!(matches!(
        db.advance_clone(&unknown, CloneProgress::Pending(ClonePhase::Reserved)),
        Err(Error::InvalidTransition)
    ));
    assert!(matches!(
        db.advance_clone(
            &unknown,
            CloneProgress::Pending(ClonePhase::Dispatched {
                identity: "replacement".into()
            })
        ),
        Err(Error::InvalidTransition)
    ));
    assert!(matches!(
        db.advance_clone(&record, record.progress.clone()),
        Err(Error::InvalidTransition)
    ));
    let journal = db.process_journal().unwrap();
    journal
        .record_outcome(attempt.intent.run, /*exit_code*/ 128)
        .unwrap();
    journal.cleaned(attempt.intent.run).unwrap();
    let result = CloneExecutionResult::CloneFailed(CloneFailed {
        node: NodeRuntimeIdentity {
            node_id: db.node_id().clone(),
            incarnation_id: NodeIncarnationId::new("original"),
        },
        spec: command.payload.spec.clone(),
        failure: CloneFailureCode::SourceUnavailable,
        residual: CloneResidual::Retained {
            repository_id: target.repository_id.clone(),
            path: NodePath::new(target.path.to_str().unwrap()),
        },
    });
    db.complete_clone(&unknown, result.clone()).unwrap();
    let mut retry = command;
    retry.operation_id = OperationId::new("retry-op");
    retry.execution_id = ExecutionId::new("retry-exec");
    assert!(matches!(
        db.accept_clone(&retry, &target),
        Err(Error::ResourceConflict)
    ));
    let retry_target = CloneTarget {
        repository_id: RepositoryId::new("repository-2"),
        path: target.root.join("repository-2"),
        ..target
    };
    assert_eq!(
        db.accept_clone(&retry, &retry_target).unwrap().progress,
        CloneProgress::Pending(ClonePhase::Reserved)
    );
    assert_eq!(db.pending_events().unwrap(), vec![unknown.event(result)]);
}

/// Failed acceptance must roll back identity registration as well as destination ownership.
#[test]
fn clone_acceptance_failure_leaves_no_identity_or_reservation() {
    let dir = tempfile::tempdir().unwrap();
    let fault = std::rc::Rc::new(std::cell::Cell::new(Some(WritePoint::Accept)));
    let mut db = NodeDatabase::open_with_guard(
        &dir.path().join("db"),
        NodeIdentity::Discover,
        Fault(fault.clone()),
    )
    .unwrap();
    let (command, target) = clone_fixture(db.node_id(), dir.path());
    assert!(matches!(
        db.accept_clone(&command, &target),
        Err(Error::Injected(WritePoint::Accept))
    ));
    assert_eq!(
        db.find_clone(&command.operation_id, &command.execution_id)
            .unwrap(),
        None
    );
    fault.set(/*val*/ None);
    let record = db.accept_clone(&command, &target).unwrap();
    let mut other = command;
    other.operation_id = OperationId::new("other");
    other.execution_id = ExecutionId::new("other");
    let mut collision = target;
    collision.path = dir.path().join("different-path");
    assert!(db.accept_clone(&other, &collision).is_err());
    assert_eq!(
        db.find_clone(&other.operation_id, &other.execution_id)
            .unwrap(),
        None
    );
    assert_eq!(db.recoverable_clones().unwrap(), vec![record]);
}
