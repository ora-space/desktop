use super::*;
use ora_node_protocol::*;
use ora_process_protocol::*;
use pretty_assertions::assert_eq;

/// Copies actual accepted Worktree state into the exact previous schema, including pending Run evidence.
#[test]
fn version_two_upgrade_preserves_live_process_responsibility_and_legacy_results() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.db");
    let path = dir.path().join("version-two.db");
    let mut db = NodeDatabase::open(&source, NodeIdentity::Discover).unwrap();
    let (command, target) = fixture(db.node_id());
    let accepted = db.accept(&command, &target).unwrap();
    let journal = db.process_journal().unwrap();
    journal.manage(&accepted).unwrap();
    let attempt = ProcessAttempt {
        execution: command.execution_id().clone(),
        host_directory: dir.path().join("original-host"),
        expected_uid: 42,
        intent: HostRunIntent {
            scope: ScopeId::new(),
            run: RunId::new(),
            spec: RunSpec::new("git", &target.main_path, DescendantPolicy::WaitForAll),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        },
    };
    journal.record(&attempt).unwrap();
    // A separate completed execution exercises unchanged terminal JSON and outbox transfer too.
    let (mut rejected, _) = fixture(db.node_id());
    let Command::Ensure(message) = &mut rejected else {
        panic!("fixture")
    };
    message.operation_id = OperationId::new("rejected-op");
    message.execution_id = ExecutionId::new("rejected-exec");
    let failure = WorktreeExecutionResult::Failed(WorktreeFailed {
        node: NodeRuntimeIdentity {
            node_id: db.node_id().clone(),
            incarnation_id: NodeIncarnationId::new("old"),
        },
        workspace_id: rejected.spec().workspace_id.clone(),
        worktree_id: rejected.spec().worktree_id.clone(),
        failure: WorktreeFailure {
            code: WorktreeFailureCode::BranchConflict,
            message: "retained failure".into(),
        },
    });
    db.reject(&rejected, failure).unwrap();
    let events = db.pending_events().unwrap();
    let legacy = Connection::open(&path).unwrap();
    legacy.execute_batch("PRAGMA application_id=1330790734; PRAGMA user_version=2;
        CREATE TABLE node_metadata (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), node_id TEXT NOT NULL CHECK(length(node_id) > 0));").unwrap();
    legacy.execute_batch(include_str!("../schema.sql")).unwrap();
    legacy
        .execute_batch(include_str!("../process.sql"))
        .unwrap();
    legacy
        .execute("ATTACH DATABASE ?1 AS original", [source.to_str().unwrap()])
        .unwrap();
    for table in [
        "node_metadata",
        "executions",
        "resources",
        "outbox",
        "managed_executions",
        "process_attempts",
    ] {
        legacy
            .execute(
                &format!("INSERT INTO {table} SELECT * FROM original.{table}"),
                [],
            )
            .unwrap();
    }
    drop(legacy);
    drop(journal);
    drop(db);
    let mut upgraded = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(upgraded.existing(&command).unwrap(), Some(accepted.clone()));
    assert_eq!(upgraded.pending_events().unwrap(), events);
    let journal = upgraded.process_journal().unwrap();
    assert_eq!(
        journal.pending(command.execution_id()).unwrap(),
        vec![attempt.clone()]
    );
    assert_eq!(upgraded.recoverable_clones().unwrap(), vec![]);
    journal.cleaned(attempt.intent.run).unwrap();
    upgraded
        .complete(&accepted, ready(&command, &target))
        .unwrap();
    drop(journal);
    drop(upgraded);
    let inspect = Connection::open(&path).unwrap();
    let version: i64 = inspect
        .pragma_query_value(
            /*schema_name*/ None,
            "user_version",
            |r| r.get(/*idx*/ 0),
        )
        .unwrap();
    assert_eq!(version, 3);
    // Previous binaries accept only versions 1/2 before touching persistent pragmas or migration.
    assert!(!matches!(version, 1 | 2));
    let violations: i64 = inspect
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(/*idx*/ 0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}
