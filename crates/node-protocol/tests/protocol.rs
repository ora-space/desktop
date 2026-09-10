use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::io;
use thiserror::Error;
use tokio::io::duplex;

#[path = "protocol/completed.rs"]
mod completed;
#[path = "protocol/rejections.rs"]
mod rejections;

#[derive(Debug, Error)]
enum TestError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Integer(#[from] std::num::TryFromIntError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Proves every Controller message survives the same public codec used by transports.
#[tokio::test]
async fn round_trips_every_controller_message() -> Result<(), TestError> {
    let messages = vec![
        ControllerToNodeMessage::Hello {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: Hello {
                controller_id: ControllerId::new("controller-1"),
                supported_versions: vec![CURRENT_PROTOCOL_VERSION],
            },
        },
        ControllerToNodeMessage::EnsureWorktree {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(RequestId::new("request-1")),
            operation_id: OperationId::new("operation-create"),
            execution_id: ExecutionId::new("execution-create"),
            payload: EnsureWorktree { spec: spec() },
        },
        ControllerToNodeMessage::RemoveWorktree {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(RequestId::new("request-2")),
            operation_id: OperationId::new("operation-remove"),
            execution_id: ExecutionId::new("execution-remove"),
            payload: RemoveWorktree { spec: spec() },
        },
        ControllerToNodeMessage::GetExecutionStatus {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("operation-create"),
            execution_id: ExecutionId::new("execution-create"),
            payload: GetExecutionStatus {
                node_id: NodeId::new("node-1"),
            },
        },
        ControllerToNodeMessage::EventAck {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("operation-create"),
            execution_id: ExecutionId::new("execution-create"),
            sequence: Sequence::new(/*value*/ 7),
            payload: EventAck {
                node_id: NodeId::new("node-1"),
            },
        },
    ];

    for expected in messages {
        assert_eq!(round_trip_controller(expected.clone()).await?, expected);
    }
    Ok(())
}

/// Proves every Node message and every terminal result shape survives the public codec.
#[tokio::test]
async fn round_trips_every_node_message() -> Result<(), TestError> {
    let ready = ready_result();
    let failed = failed_result();
    let removed = removed_result();
    let removal_failed = removal_failed_result();
    let messages = vec![
        NodeToControllerMessage::HelloAccepted {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: HelloAccepted {
                selected_version: CURRENT_PROTOCOL_VERSION,
                node: node(),
                capabilities: vec![NodeCapability::WorktreeExecution],
            },
        },
        NodeToControllerMessage::Heartbeat {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: Heartbeat { node: node() },
        },
        NodeToControllerMessage::ExecutionStatus {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("operation-create"),
            execution_id: ExecutionId::new("execution-create"),
            payload: ExecutionStatus {
                node: node(),
                state: ExecutionState::Completed(WorktreeExecutionResult::Ready(ready.clone())),
            },
        },
        NodeToControllerMessage::ExecutionStatus {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("operation-create"),
            execution_id: ExecutionId::new("execution-create"),
            payload: ExecutionStatus {
                node: node(),
                state: ExecutionState::Unknown,
            },
        },
        NodeToControllerMessage::ExecutionStatus {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("operation-create"),
            execution_id: ExecutionId::new("execution-create"),
            payload: ExecutionStatus {
                node: node(),
                state: ExecutionState::Accepted,
            },
        },
        NodeToControllerMessage::ExecutionStatus {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("operation-create"),
            execution_id: ExecutionId::new("execution-create"),
            payload: ExecutionStatus {
                node: node(),
                state: ExecutionState::Running,
            },
        },
        NodeToControllerMessage::WorktreeReady {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(RequestId::new("request-1")),
            operation_id: OperationId::new("operation-create"),
            execution_id: ExecutionId::new("execution-create"),
            sequence: Sequence::new(/*value*/ 1),
            payload: ready,
        },
        NodeToControllerMessage::WorktreeFailed {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(RequestId::new("request-1")),
            operation_id: OperationId::new("operation-create"),
            execution_id: ExecutionId::new("execution-create"),
            sequence: Sequence::new(/*value*/ 1),
            payload: failed,
        },
        NodeToControllerMessage::WorktreeRemoved {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(RequestId::new("request-2")),
            operation_id: OperationId::new("operation-remove"),
            execution_id: ExecutionId::new("execution-remove"),
            sequence: Sequence::new(/*value*/ 2),
            payload: removed,
        },
        NodeToControllerMessage::WorktreeRemovalFailed {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            request_id: Some(RequestId::new("request-2")),
            operation_id: OperationId::new("operation-remove"),
            execution_id: ExecutionId::new("execution-remove"),
            sequence: Sequence::new(/*value*/ 2),
            payload: removal_failed,
        },
    ];

    for expected in messages {
        assert_eq!(round_trip_node(expected.clone()).await?, expected);
    }
    Ok(())
}

/// Verifies an empty stream is the only input reported as a clean connection close.
#[tokio::test]
async fn returns_none_at_clean_eof() -> Result<(), FrameError> {
    let mut reader = tokio::io::empty();

    assert_eq!(read_controller_message(&mut reader).await?, None);
    Ok(())
}

/// Keeps truncated headers distinct from clean EOF for connection recovery decisions.
#[tokio::test]
async fn rejects_truncated_header() {
    let mut reader = std::io::Cursor::new(vec![0, 0]);

    let error = match read_controller_message(&mut reader).await {
        Err(error) => error,
        Ok(message) => panic!("partial header unexpectedly decoded as {message:?}"),
    };
    assert_io_kind(error, io::ErrorKind::UnexpectedEof);
}

/// Rejects a frame whose declared payload ends before all bytes arrive.
#[tokio::test]
async fn rejects_truncated_payload() {
    let mut reader = std::io::Cursor::new(vec![0, 0, 0, 4, NODE_MESSAGE_FRAME_TYPE, b'{']);

    let error = match read_controller_message(&mut reader).await {
        Err(error) => error,
        Ok(message) => panic!("partial payload unexpectedly decoded as {message:?}"),
    };
    assert_io_kind(error, io::ErrorKind::UnexpectedEof);
}

/// Rejects zero-length frames before attempting to read a type byte.
#[tokio::test]
async fn rejects_zero_length_frame() {
    let mut reader = std::io::Cursor::new(vec![0, 0, 0, 0]);

    assert!(matches!(
        read_controller_message(&mut reader).await,
        Err(FrameError::InvalidLength { length: 0 })
    ));
}

/// Rejects oversized frames before allocating their declared payload.
#[tokio::test]
async fn rejects_oversized_frame() -> Result<(), std::num::TryFromIntError> {
    let oversized = u32::try_from(MAX_FRAME_LENGTH + 1)?;
    let mut reader = std::io::Cursor::new(oversized.to_be_bytes().to_vec());

    assert!(matches!(
        read_controller_message(&mut reader).await,
        Err(FrameError::InvalidLength { length }) if length == MAX_FRAME_LENGTH + 1
    ));
    Ok(())
}

/// Rejects unknown frame categories without interpreting their bytes as protocol JSON.
#[tokio::test]
async fn rejects_unknown_frame_type() -> Result<(), std::num::TryFromIntError> {
    let mut reader = std::io::Cursor::new(framed(/*frame_type*/ 0xff, br#"{}"#)?);

    assert!(matches!(
        read_controller_message(&mut reader).await,
        Err(FrameError::UnsupportedFrameType { frame_type: 0xff })
    ));
    Ok(())
}

/// Distinguishes malformed JSON from a structurally valid message that violates invariants.
#[tokio::test]
async fn distinguishes_malformed_json_from_invalid_message() -> Result<(), TestError> {
    let mut malformed = std::io::Cursor::new(framed(NODE_MESSAGE_FRAME_TYPE, br#"{"#)?);
    assert!(matches!(
        read_controller_message(&mut malformed).await,
        Err(FrameError::DecodeJson(_))
    ));

    let invalid = ControllerToNodeMessage::EnsureWorktree {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: None,
        operation_id: OperationId::new(""),
        execution_id: ExecutionId::new("execution-create"),
        payload: EnsureWorktree { spec: spec() },
    };
    let json = serde_json::to_vec(&invalid)?;
    let mut invalid = std::io::Cursor::new(framed(NODE_MESSAGE_FRAME_TYPE, &json)?);
    assert!(matches!(
        read_controller_message(&mut invalid).await,
        Err(FrameError::InvalidMessage(_))
    ));
    Ok(())
}

/// Direction-specific decoding refuses a valid message sent by the opposite peer role.
#[tokio::test]
async fn rejects_wrong_direction_message() -> Result<(), TestError> {
    let message = ControllerToNodeMessage::Hello {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Hello {
            controller_id: ControllerId::new("controller-1"),
            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
        },
    };
    let (mut writer, mut reader) = duplex(/*max_buf_size*/ 64);
    let write_task =
        tokio::spawn(async move { write_controller_message(&mut writer, &message).await });

    assert!(matches!(
        read_node_message(&mut reader).await,
        Err(FrameError::DecodeJson(_))
    ));
    write_task.await??;
    Ok(())
}

/// Keeps every occurrence of an identity in the same transparent JSON representation.
#[test]
fn serializes_identity_consistently_across_messages() -> Result<(), serde_json::Error> {
    let operation_id = OperationId::new("operation-1");
    let controller = ControllerToNodeMessage::GetExecutionStatus {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: operation_id.clone(),
        execution_id: ExecutionId::new("execution-1"),
        payload: GetExecutionStatus {
            node_id: NodeId::new("node-1"),
        },
    };
    let node = NodeToControllerMessage::ExecutionStatus {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id,
        execution_id: ExecutionId::new("execution-1"),
        payload: ExecutionStatus {
            node: node(),
            state: ExecutionState::Running,
        },
    };

    let controller_json = serde_json::to_value(controller)?;
    let node_json = serde_json::to_value(node)?;
    assert_eq!(controller_json["operation_id"], node_json["operation_id"]);
    Ok(())
}

/// Locks the first-version envelope field names independently from Rust enum representation.
#[test]
fn serializes_the_documented_wire_envelope() -> Result<(), serde_json::Error> {
    let message = ControllerToNodeMessage::Hello {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Hello {
            controller_id: ControllerId::new("controller-1"),
            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
        },
    };

    assert_eq!(
        serde_json::to_value(message)?,
        json!({
            "message_type": "hello",
            "protocol_version": 1,
            "payload": {
                "controller_id": "controller-1",
                "supported_versions": [1]
            }
        })
    );
    Ok(())
}

/// Forces a Controller message through a three-byte stream buffer to fragment its frame.
async fn round_trip_controller(
    expected: ControllerToNodeMessage,
) -> Result<ControllerToNodeMessage, TestError> {
    let (mut writer, mut reader) = duplex(/*max_buf_size*/ 3);
    let message = expected.clone();
    let write_task =
        tokio::spawn(async move { write_controller_message(&mut writer, &message).await });
    let actual = read_controller_message(&mut reader).await?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "round trip did not contain a Controller message",
        )
    })?;
    write_task.await??;
    Ok(actual)
}

/// Forces a Node message through a three-byte stream buffer to fragment its frame.
async fn round_trip_node(
    expected: NodeToControllerMessage,
) -> Result<NodeToControllerMessage, TestError> {
    let (mut writer, mut reader) = duplex(/*max_buf_size*/ 3);
    let message = expected.clone();
    let write_task = tokio::spawn(async move { write_node_message(&mut writer, &message).await });
    let actual = read_node_message(&mut reader).await?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "round trip did not contain a Node message",
        )
    })?;
    write_task.await??;
    Ok(actual)
}

/// Builds raw input for negative framing and decoding tests.
fn framed(frame_type: u8, payload: &[u8]) -> Result<Vec<u8>, std::num::TryFromIntError> {
    let length = u32::try_from(payload.len() + 1)?;
    let mut frame = Vec::with_capacity(payload.len() + 5);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.push(frame_type);
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Compares the preserved I/O category without relying on formatted error text.
fn assert_io_kind(error: FrameError, expected: io::ErrorKind) {
    match error {
        FrameError::Io(source) => assert_eq!(source.kind(), expected),
        other => panic!("expected I/O error, got {other:?}"),
    }
}

/// Returns one complete normalized Worktree execution input shared by command fixtures.
fn spec() -> WorktreeExecutionSpec {
    WorktreeExecutionSpec {
        node_id: NodeId::new("node-1"),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        repository: RepositoryRef::new("repository-1"),
        main_workspace: MainWorkspaceBinding {
            workspace_id: WorkspaceId::new("workspace-main"),
            path: NodePath::new("/node/repos/ora"),
        },
        base_ref: GitRef::new("refs/heads/main"),
        expected_branch: BranchName::new("ora/12345678"),
        path_policy: WorktreePathPolicy::NodeManaged {
            directory_name: "workspace-task".to_string(),
        },
    }
}

/// Returns the Node identity attached to every result fixture.
fn node() -> NodeRuntimeIdentity {
    NodeRuntimeIdentity {
        node_id: NodeId::new("node-1"),
        incarnation_id: NodeIncarnationId::new("incarnation-1"),
    }
}

/// Returns a successful creation result containing every Node-scoped fact.
fn ready_result() -> WorktreeReady {
    WorktreeReady {
        node: node(),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        facts: WorktreeFacts {
            path: NodePath::new("/node/worktrees/workspace-task"),
            branch: BranchName::new("ora/12345678"),
            base_commit: CommitId::new("0123456789abcdef"),
        },
    }
}

/// Returns one structured creation failure.
fn failed_result() -> WorktreeFailed {
    WorktreeFailed {
        node: node(),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        failure: WorktreeFailure {
            code: WorktreeFailureCode::BranchConflict,
            message: "branch is already checked out".to_string(),
        },
    }
}

/// Returns the idempotent already-absent removal outcome.
fn removed_result() -> WorktreeRemoved {
    WorktreeRemoved {
        node: node(),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        outcome: WorktreeRemovalOutcome::AlreadyAbsent,
    }
}

/// Returns one structured removal failure.
fn removal_failed_result() -> WorktreeRemovalFailed {
    WorktreeRemovalFailed {
        node: node(),
        workspace_id: WorkspaceId::new("workspace-task"),
        worktree_id: WorktreeId::new("worktree-1"),
        failure: WorktreeFailure {
            code: WorktreeFailureCode::OperationFailed,
            message: "Git refused to remove the worktree".to_string(),
        },
    }
}
