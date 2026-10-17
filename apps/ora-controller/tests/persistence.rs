#![allow(clippy::unwrap_used)]
#[path = "support/local_clone.rs"]
mod local_clone;
use ora_controller::*;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use std::sync::{Arc, Mutex};

struct Fault(Arc<Mutex<Option<WritePoint>>>);
impl WriteGuard for Fault {
    /// Refuses one transaction boundary while preserving the production database behavior.
    fn before_write(&self, point: WritePoint) -> Result<(), Error> {
        if *self.0.lock().unwrap() == Some(point) {
            Err(Error::Injected)
        } else {
            Ok(())
        }
    }
}

/// Uses an owner-private deployment location rather than weakening production path policy for tests.
fn directory() -> tempfile::TempDir {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(std::env::var_os("HOME").unwrap())
            .unwrap()
    }
    #[cfg(not(unix))]
    {
        tempfile::tempdir().unwrap()
    }
}

/// Builds retained historical facts with a different incarnation from the current session.
fn event(command: &CloneRepositoryMessage) -> CloneResultMessage {
    CloneResultMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: command.request_id.clone(),
        operation_id: command.operation_id.clone(),
        execution_id: command.execution_id.clone(),
        sequence: Sequence::new(/*value*/ 1),
        payload: CloneExecutionResult::CloneReady(CloneReady {
            node: NodeRuntimeIdentity {
                node_id: NodeId::new("node"),
                incarnation_id: NodeIncarnationId::new("historical"),
            },
            spec: command.payload.spec.clone(),
            repository_id: RepositoryId::new("repository"),
            path: NodePath::new("/node/checkout"),
            commit: CommitId::new(local_clone::BASE_COMMIT),
        }),
    }
}

/// A held dispatch survives restart, and the database excludes a second live owner and an owner
/// with another identity.
#[tokio::test]
async fn held_dispatches_are_durable_and_ownership_exclusive() {
    let directory = directory();
    let owner = SqliteStore::open(directory.path(), ControllerId::new("owner")).unwrap();
    let held = local_clone::seed(directory.path(), "request");
    assert!(matches!(
        SqliteStore::open(directory.path(), ControllerId::new("owner")),
        Err(Error::AlreadyRunning)
    ));
    drop(owner);
    let owner = SqliteStore::open(directory.path(), ControllerId::new("owner")).unwrap();
    assert_eq!(
        owner
            .pending_dispatches(&NodeId::new("node"))
            .await
            .unwrap(),
        vec![held]
    );
    drop(owner);
    assert!(matches!(
        SqliteStore::open(directory.path(), ControllerId::new("other")),
        Err(Error::InvalidStorage)
    ));
}

/// Query/event order and lost Ack share one immutable result; receipt failure rolls back first takeover.
#[tokio::test]
async fn takeover_is_atomic_in_both_delivery_orders_and_conflicts_never_ack() {
    for query_first in [true, false] {
        let directory = directory();
        let fault = Arc::new(Mutex::new(None));
        let owner = SqliteStore::open_with_guard(
            directory.path(),
            ControllerId::new("owner"),
            Fault(fault.clone()),
        )
        .unwrap();
        let command = local_clone::seed(directory.path(), "request");
        let event = event(&command);
        let session = NodeRuntimeIdentity {
            node_id: NodeId::new("node"),
            incarnation_id: NodeIncarnationId::new("current"),
        };
        let query = NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: command.operation_id.clone(),
            execution_id: command.execution_id.clone(),
            payload: ExecutionStatus {
                node: session.clone(),
                state: ExecutionState::Completed(ExecutionResult::Clone(Box::new(
                    event.payload.clone(),
                ))),
            },
        });
        *fault.lock().unwrap() = Some(WritePoint::Receipt);
        assert!(matches!(
            take_over(
                &owner,
                &session,
                &NodeToControllerMessage::CloneResult(event.clone())
            )
            .await,
            Err(Error::Injected)
        ));
        assert_eq!(owner.result(&command.execution_id).await.unwrap(), None);
        assert_eq!(
            owner
                .pending_dispatches(&NodeId::new("node"))
                .await
                .unwrap(),
            vec![command.clone()]
        );
        *fault.lock().unwrap() = None;
        if query_first {
            assert_eq!(take_over(&owner, &session, &query).await.unwrap(), None);
        }
        let expected = Some(EventAckMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: command.operation_id.clone(),
            execution_id: command.execution_id.clone(),
            sequence: event.sequence,
            payload: EventAck {
                node_id: session.node_id.clone(),
            },
        });
        assert_eq!(
            take_over(
                &owner,
                &session,
                &NodeToControllerMessage::CloneResult(event.clone())
            )
            .await
            .unwrap(),
            expected
        );
        assert_eq!(take_over(&owner, &session, &query).await.unwrap(), None);
        // A committed result retires the execution from periodic queries; replayed events are
        // still recognized through the original dispatch.
        assert_eq!(
            owner
                .pending_dispatches(&NodeId::new("node"))
                .await
                .unwrap(),
            vec![]
        );
        drop(owner);
        let owner = SqliteStore::open(directory.path(), ControllerId::new("owner")).unwrap();
        assert_eq!(
            take_over(
                &owner,
                &session,
                &NodeToControllerMessage::CloneResult(event.clone())
            )
            .await
            .unwrap(),
            expected
        );
        let mut wrong = event.clone();
        wrong.request_id = Some(RequestId::new("other"));
        assert!(matches!(
            take_over(
                &owner,
                &session,
                &NodeToControllerMessage::CloneResult(wrong)
            )
            .await,
            Err(Error::Conflict)
        ));
        // A checkout at another commit than the snapshot's base is never taken over or acknowledged.
        let mut wrong = event.clone();
        if let CloneExecutionResult::CloneReady(ready) = &mut wrong.payload {
            ready.commit = CommitId::new("1123456789abcdef0123456789abcdef01234567");
        }
        assert!(matches!(
            take_over(
                &owner,
                &session,
                &NodeToControllerMessage::CloneResult(wrong)
            )
            .await,
            Err(Error::Validation(_))
        ));
        assert_eq!(
            owner.result(&command.execution_id).await.unwrap(),
            Some(ExecutionOutcome::from(&event.payload))
        );
        let inspect =
            rusqlite::Connection::open(directory.path().join("ora-controller.sqlite3")).unwrap();
        assert_eq!(
            inspect
                .query_row("SELECT input,result FROM clone_operations", [], |r| Ok((
                    r.get::<_, String>(/*idx*/ 0)?,
                    r.get::<_, String>(/*idx*/ 1)?
                )))
                .map(|(input, result)| (
                    serde_json::from_str::<CloneRepositoryMessage>(&input).unwrap(),
                    serde_json::from_str::<CloneExecutionResult>(&result).unwrap()
                ))
                .unwrap(),
            (command.clone(), event.payload.clone())
        );
        assert_eq!(
            inspect
                .query_row("SELECT count(*) FROM clone_receipts", [], |r| r
                    .get::<_, i64>(/*idx*/ 0))
                .unwrap(),
            1
        );
    }
}

/// Foreign files are not reinitialized.
#[tokio::test]
async fn unknown_files_remain_untouched() {
    let directory = directory();
    let foreign = directory.path().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(/*mode*/ 0o700))
            .unwrap();
    }
    let path = foreign.join("ora-controller.sqlite3");
    std::fs::write(&path, "user content").unwrap();
    assert!(SqliteStore::open(&foreign, ControllerId::new("owner")).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "user content");
}
