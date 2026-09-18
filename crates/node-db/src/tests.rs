#![allow(clippy::unwrap_used)]
use super::*;
use pretty_assertions::assert_eq;
mod process;
mod repository;
mod repository_migration;

/// Reopening preserves identity, while one live owner excludes all other connections.
#[test]
fn identity_and_exclusive_owner_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ora-node.sqlite3");
    assert!(matches!(
        NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new(" "))),
        Err(Error::NodeMismatch)
    ));
    assert!(!path.exists());
    let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    let id = db.node_id().clone();
    assert!(matches!(
        NodeDatabase::open(&path, NodeIdentity::Discover),
        Err(Error::AlreadyRunning)
    ));
    drop(db);
    assert!(matches!(
        NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("wrong"))),
        Err(Error::NodeMismatch)
    ));
    let reopened = NodeDatabase::open(&path, NodeIdentity::Require(id.clone())).unwrap();
    assert_eq!(reopened.node_id(), &id);
}

/// Invalid preexisting storage must remain byte-for-byte intact.
#[test]
fn preserves_foreign_corrupt_empty_and_future_files() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["foreign", "corrupt", "empty", "future"] {
        let path = dir.path().join(name);
        match name {
            "foreign" => {
                Connection::open(&path)
                    .unwrap()
                    .execute_batch("CREATE TABLE other (value TEXT);")
                    .unwrap();
            }
            "future" => {
                drop(NodeDatabase::open(&path, NodeIdentity::Discover).unwrap());
                Connection::open(&path)
                    .unwrap()
                    .pragma_update(
                        /*schema_name*/ None,
                        "user_version",
                        /*pragma_value*/ 999,
                    )
                    .unwrap();
            }
            "corrupt" => std::fs::write(&path, b"not sqlite").unwrap(),
            "empty" => std::fs::write(&path, b"").unwrap(),
            _ => unreachable!(),
        }
        let before = std::fs::read(&path).unwrap();
        assert!(NodeDatabase::open(&path, NodeIdentity::Discover).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    assert!(NodeDatabase::open(dir.path(), NodeIdentity::Discover).is_err());
}

/// Creates a complete request/target fixture without relying on a live repository.
fn fixture(node: &NodeId) -> (Command, Target) {
    use ora_node_protocol::*;
    let main = std::env::temp_dir().join("node-db-main");
    let root = std::env::temp_dir().join("node-db-worktrees");
    let spec = WorktreeExecutionSpec {
        node_id: node.clone(),
        workspace_id: WorkspaceId::new("task"),
        worktree_id: WorktreeId::new("tree"),
        repository: RepositoryRef::new("repo"),
        main_workspace: MainWorkspaceBinding {
            workspace_id: WorkspaceId::new("main"),
            path: NodePath::new(main.to_str().unwrap()),
        },
        base_ref: GitRef::new("main"),
        expected_branch: BranchName::new("ora/task"),
        path_policy: WorktreePathPolicy::NodeManaged {
            directory_name: "task".into(),
        },
    };
    let target = Target {
        main_path: main.clone(),
        git_directory: main.join(".git"),
        authorized_root: std::env::temp_dir(),
        worktree_root: root.clone(),
        path: root.join("task"),
        branch: spec.expected_branch.clone(),
        base_commit: CommitId::new("abc"),
    };
    (
        Command::Ensure(EnsureWorktreeMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(RequestId::new("request")),
            operation_id: OperationId::new("operation"),
            execution_id: ExecutionId::new("execution"),
            payload: EnsureWorktree { spec },
        }),
        target,
    )
}

/// Creates an attributable terminal result to exercise atomic result and event storage.
fn ready(command: &Command, target: &Target) -> ora_node_protocol::WorktreeExecutionResult {
    use ora_node_protocol::*;
    WorktreeExecutionResult::Ready(WorktreeReady {
        node: NodeRuntimeIdentity {
            node_id: command.spec().node_id.clone(),
            incarnation_id: NodeIncarnationId::new("incarnation"),
        },
        workspace_id: command.spec().workspace_id.clone(),
        worktree_id: command.spec().worktree_id.clone(),
        facts: WorktreeFacts {
            path: NodePath::new(target.path.to_str().unwrap()),
            branch: target.branch.clone(),
            base_commit: target.base_commit.clone(),
        },
    })
}

/// Identity uniqueness and reservations survive reconnects and reject all rebindings.
#[test]
fn atomic_acceptance_deduplicates_and_reserves_resources() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    let (command, target) = fixture(db.node_id());
    let accepted = db.accept(&command, &target).unwrap();
    assert_eq!(db.accept(&command, &target).unwrap(), accepted);
    let Command::Ensure(mut changed) = command.clone() else {
        unreachable!()
    };
    changed.execution_id = ora_node_protocol::ExecutionId::new("other");
    assert!(matches!(
        db.accept(&Command::Ensure(changed.clone()), &target),
        Err(Error::IdentityConflict)
    ));
    changed.operation_id = ora_node_protocol::OperationId::new("other");
    assert!(matches!(
        db.accept(&Command::Ensure(changed), &target),
        Err(Error::ResourceConflict)
    ));
    let Command::Ensure(mut changed) = command.clone() else {
        unreachable!()
    };
    changed.payload.spec.base_ref = ora_node_protocol::GitRef::new("moved");
    assert!(matches!(
        db.existing(&Command::Ensure(changed)),
        Err(Error::IdentityConflict)
    ));
    drop(db);
    let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(db.recoverable().unwrap(), vec![accepted]);
}

/// Faults are armed after initialization so tests reach the exact transactional boundary.
struct Fault(std::rc::Rc<std::cell::Cell<Option<WritePoint>>>);
impl WriteGuard for Fault {
    /// Fails only the requested write without faking SQLite persistence.
    fn before_write(&self, point: WritePoint) -> Result<(), Error> {
        if self.0.get() == Some(point) {
            Err(Error::Injected(point))
        } else {
            Ok(())
        }
    }
}

/// An outbox failure rolls back both terminal state and ownership facts, then reopening can finish.
#[test]
fn completion_and_outbox_rollback_together_and_ack_retains_result() {
    use ora_node_protocol::*;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let fault = std::rc::Rc::new(std::cell::Cell::new(/*value*/ None));
    let mut db =
        NodeDatabase::open_with_guard(&path, NodeIdentity::Discover, Fault(fault.clone())).unwrap();
    let (command, target) = fixture(db.node_id());
    fault.set(Some(WritePoint::Accept));
    assert!(db.accept(&command, &target).is_err());
    assert_eq!(db.existing(&command).unwrap(), None);
    fault.set(/*val*/ None);
    let accepted = db.accept(&command, &target).unwrap();
    let running = Progress::Running {
        stage: Stage::Create,
        observer: NodeRuntimeIdentity {
            node_id: db.node_id().clone(),
            incarnation_id: NodeIncarnationId::new("one"),
        },
        observed_at: "local time".into(),
    };
    fault.set(Some(WritePoint::Progress));
    assert!(db.advance(&accepted, running.clone()).is_err());
    assert_eq!(db.existing(&command).unwrap(), Some(accepted.clone()));
    fault.set(/*val*/ None);
    let record = db.advance(&accepted, running).unwrap();
    let result = ready(&command, &target);
    let resource = db.resource(&command.spec().worktree_id).unwrap();
    fault.set(Some(WritePoint::Outbox));
    assert!(db.complete(&record, result.clone()).is_err());
    assert_eq!(db.existing(&command).unwrap(), Some(record.clone()));
    assert_eq!(db.resource(&command.spec().worktree_id).unwrap(), resource);
    assert_eq!(db.pending_events().unwrap(), vec![]);
    drop(db);
    let mut db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    db.complete(&record, result.clone()).unwrap();
    assert_eq!(
        db.pending_events().unwrap(),
        vec![command.event(result.clone())]
    );
    let mut ack = EventAckMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command.operation_id().clone(),
        execution_id: command.execution_id().clone(),
        sequence: Sequence::new(/*value*/ 2),
        payload: EventAck {
            node_id: db.node_id().clone(),
        },
    };
    assert!(db.acknowledge(&ack).is_err());
    assert_eq!(
        db.pending_events().unwrap(),
        vec![command.event(result.clone())]
    );
    ack.sequence = Sequence::new(/*value*/ 1);
    db.acknowledge(&ack).unwrap();
    db.acknowledge(&ack).unwrap();
    drop(db);
    let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    assert_eq!(db.pending_events().unwrap(), vec![]);
    assert_eq!(
        db.existing(&command).unwrap().unwrap().progress,
        Progress::Completed { result }
    );
}

/// Even a recognizable application/version header cannot authorize missing tables or relaxed constraints.
#[test]
fn rejects_modified_schema_without_migrating_or_rebuilding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    drop(NodeDatabase::open(&path, NodeIdentity::Discover).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP INDEX resource_path;")
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        NodeDatabase::open(&path, NodeIdentity::Discover),
        Err(Error::InvalidSchema)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

/// Invalid terminal and observer transitions cannot corrupt input ownership or seal an unknown result.
#[test]
fn rejects_unattributable_progress_and_unresolved_terminal_results() {
    use ora_node_protocol::*;
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(&dir.path().join("db"), NodeIdentity::Discover).unwrap();
    let (command, target) = fixture(db.node_id());
    assert!(matches!(
        db.reject(&command, ready(&command, &target)),
        Err(Error::InvalidTransition)
    ));
    let record = db.accept(&command, &target).unwrap();
    let observer = NodeRuntimeIdentity {
        node_id: NodeId::new("wrong"),
        incarnation_id: NodeIncarnationId::new("run"),
    };
    assert!(matches!(
        db.advance(
            &record,
            Progress::Running {
                stage: Stage::Create,
                observer,
                observed_at: "local".into()
            }
        ),
        Err(Error::InvalidTransition)
    ));
    let result = WorktreeExecutionResult::Failed(WorktreeFailed {
        node: NodeRuntimeIdentity {
            node_id: db.node_id().clone(),
            incarnation_id: NodeIncarnationId::new("run"),
        },
        workspace_id: command.spec().workspace_id.clone(),
        worktree_id: command.spec().worktree_id.clone(),
        failure: WorktreeFailure {
            code: WorktreeFailureCode::ResultUnknown,
            message: "not proven".into(),
        },
    });
    assert!(matches!(
        db.complete(&record, result),
        Err(Error::InvalidTransition)
    ));
    assert_eq!(db.existing(&command).unwrap(), Some(record));
    assert_eq!(db.pending_events().unwrap(), vec![]);
}

/// A definitive no-effect failure frees capacity for a new resource while retaining old deduplication.
#[test]
fn definitive_create_failure_retires_its_reservation() {
    use ora_node_protocol::*;
    let dir = tempfile::tempdir().unwrap();
    let mut db = NodeDatabase::open(&dir.path().join("db"), NodeIdentity::Discover).unwrap();
    let (command, target) = fixture(db.node_id());
    let record = db.accept(&command, &target).unwrap();
    let result = WorktreeExecutionResult::Failed(WorktreeFailed {
        node: NodeRuntimeIdentity {
            node_id: db.node_id().clone(),
            incarnation_id: NodeIncarnationId::new("run"),
        },
        workspace_id: command.spec().workspace_id.clone(),
        worktree_id: command.spec().worktree_id.clone(),
        failure: WorktreeFailure {
            code: WorktreeFailureCode::OperationFailed,
            message: "no effects remain".into(),
        },
    });
    db.complete(&record, result.clone()).unwrap();
    let Command::Ensure(mut fresh) = command.clone() else {
        unreachable!()
    };
    fresh.operation_id = OperationId::new("new-operation");
    fresh.execution_id = ExecutionId::new("new-execution");
    fresh.payload.spec.worktree_id = WorktreeId::new("new-tree");
    assert!(db.accept(&Command::Ensure(fresh), &target).is_ok());
    assert_eq!(
        db.existing(&command).unwrap().unwrap().progress,
        Progress::Completed { result }
    );
}
