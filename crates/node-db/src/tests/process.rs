use super::*;
use ora_node_protocol::*;
use ora_process_protocol::*;
use pretty_assertions::assert_eq;

/// Process associations share the Node lease, survive reopen and retain original host paths and Run identities.
#[test]
fn process_journal_persists_before_dispatch_and_holds_the_original_lease() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    let (command, target) = fixture(db.node_id());
    let accepted = db.accept(&command, &target).unwrap();
    let journal = db.process_journal().unwrap();
    journal.manage(&accepted).unwrap();
    let mut spec = RunSpec::new("git", &target.main_path, DescendantPolicy::WaitForAll);
    spec.env.insert("EXPLICIT".into(), "private-value".into());
    let attempt = ProcessAttempt {
        execution: command.execution_id().clone(),
        host_directory: dir.path().join("original-host"),
        expected_uid: 42,
        intent: HostRunIntent {
            scope: ScopeId::new(),
            run: RunId::new(),
            spec,
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        },
    };
    journal.record(&attempt).unwrap();
    assert!(journal.record(&attempt).is_err());
    drop(db);
    assert!(matches!(
        NodeDatabase::open(&path, NodeIdentity::Discover),
        Err(Error::AlreadyRunning)
    ));
    drop(journal);
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    let journal = db.process_journal().unwrap();
    assert_eq!(
        journal.pending(command.execution_id()).unwrap(),
        vec![attempt.clone()]
    );
    assert_eq!(
        journal.pending_executions().unwrap(),
        vec![command.execution_id().clone()]
    );
    journal.cleaned(attempt.intent.run).unwrap();
    assert_eq!(journal.pending(command.execution_id()).unwrap(), vec![]);
    db.complete(&accepted, ready(&command, &target)).unwrap();
    let mut other = attempt;
    other.intent.run = RunId::new();
    assert!(matches!(
        journal.record(&other),
        Err(Error::InvalidTransition)
    ));
}

/// Failure to persist association grants no dispatch permission; retry retains the accepted execution.
#[test]
fn process_registration_failure_and_legacy_running_never_claim_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let fault = std::rc::Rc::new(std::cell::Cell::new(None));
    let mut db = NodeDatabase::open_with_guard(
        &dir.path().join("db"),
        NodeIdentity::Discover,
        Fault(fault.clone()),
    )
    .unwrap();
    let (command, target) = fixture(db.node_id());
    let accepted = db.accept(&command, &target).unwrap();
    let journal = db.process_journal().unwrap();
    fault.set(Some(WritePoint::Process));
    assert!(matches!(
        journal.manage(&accepted),
        Err(Error::Injected(WritePoint::Process))
    ));
    assert_eq!(db.existing(&command).unwrap(), Some(accepted.clone()));
    fault.set(/*val*/ None);
    let running = db
        .advance(
            &accepted,
            Progress::Running {
                stage: Stage::Create,
                observer: NodeRuntimeIdentity {
                    node_id: db.node_id().clone(),
                    incarnation_id: NodeIncarnationId::new("old"),
                },
                observed_at: "2026-09-17T12:00:00+08:00".into(),
            },
        )
        .unwrap();
    assert!(matches!(
        journal.manage(&running),
        Err(Error::InvalidTransition)
    ));
    assert!(matches!(
        journal.manage(&accepted),
        Err(Error::InvalidTransition)
    ));
    assert_eq!(journal.pending_executions().unwrap(), vec![]);
}

/// Schema v1 upgrades retain original identities, results and outbox while adding no fabricated process evidence.
#[test]
fn exact_version_one_upgrade_preserves_execution_and_event_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    let (command, target) = fixture(db.node_id());
    let accepted = db.accept(&command, &target).unwrap();
    db.complete(&accepted, ready(&command, &target)).unwrap();
    let events = db.pending_events().unwrap();
    drop(db);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE process_outcomes; DROP TABLE process_attempts; DROP TABLE managed_executions;
             DROP TABLE clone_outbox; DROP TABLE clone_executions; DROP TRIGGER register_worktree_execution;
             DROP TABLE execution_identities; PRAGMA user_version=1;",
        )
        .unwrap();
    drop(connection);
    let db =
        NodeDatabase::open(&path, NodeIdentity::Require(command.spec().node_id.clone())).unwrap();
    assert_eq!(db.pending_events().unwrap(), events);
    assert_eq!(
        db.process_journal().unwrap().pending_executions().unwrap(),
        vec![]
    );
    assert_eq!(
        db.existing(&command).unwrap().unwrap().progress,
        Progress::Completed {
            result: ready(&command, &target)
        }
    );
}
