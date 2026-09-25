use super::support::*;
use super::worktree::spec;
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use std::io;

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

    let invalid = ControllerToNodeMessage::EnsureWorktree(EnsureWorktreeMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: None,
        operation_id: OperationId::new(""),
        execution_id: ExecutionId::new("execution-create"),
        payload: EnsureWorktree { spec: spec() },
    });
    let json = serde_json::to_vec(&invalid)?;
    let mut invalid = std::io::Cursor::new(framed(NODE_MESSAGE_FRAME_TYPE, &json)?);
    assert!(matches!(
        read_controller_message(&mut invalid).await,
        Err(FrameError::InvalidMessage(_))
    ));
    Ok(())
}

/// Rejects an oversized serialized message before writing its length header.
#[tokio::test]
async fn rejects_oversized_outbound_message_without_writing() -> Result<(), TestError> {
    let message = ControllerToNodeMessage::Hello(HelloMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Hello {
            controller_id: ControllerId::new("x".repeat(MAX_FRAME_LENGTH)),
            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
        },
    });
    let mut output = Vec::new();
    assert!(matches!(
        write_controller_message(&mut output, &message).await,
        Err(FrameError::InvalidLength { .. })
    ));
    assert_eq!(output, Vec::<u8>::new());
    Ok(())
}

/// The stream envelope is exactly a length prefix in front of the frame body, so IPC wire bytes
/// stay unchanged while message-oriented transports carry the identical body.
#[tokio::test]
async fn stream_envelope_is_length_prefix_around_frame_body() -> Result<(), TestError> {
    let controller = ControllerToNodeMessage::Hello(HelloMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Hello {
            controller_id: ControllerId::new("owner"),
            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
        },
    });
    let body = encode_controller_frame(&controller)?;
    let mut wire = Vec::new();
    write_controller_message(&mut wire, &controller).await?;
    assert_eq!(wire, framed(NODE_MESSAGE_FRAME_TYPE, &body[1..])?);
    assert_eq!(decode_controller_frame(&body)?, controller);

    let node = NodeToControllerMessage::Heartbeat(HeartbeatMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Heartbeat {
            node: NodeRuntimeIdentity {
                node_id: NodeId::new("node"),
                incarnation_id: NodeIncarnationId::new("incarnation"),
            },
        },
    });
    let body = encode_node_frame(&node)?;
    let mut wire = Vec::new();
    write_node_message(&mut wire, &node).await?;
    assert_eq!(wire, framed(NODE_MESSAGE_FRAME_TYPE, &body[1..])?);
    assert_eq!(decode_node_frame(&body)?, node);
    Ok(())
}

/// Complete bodies are rejected on the same length, type, JSON and invariant rules as streams.
#[test]
fn rejects_invalid_frame_bodies() -> Result<(), TestError> {
    assert!(matches!(
        decode_controller_frame(&[]),
        Err(FrameError::InvalidLength { length: 0 })
    ));
    assert!(matches!(
        decode_controller_frame(&vec![NODE_MESSAGE_FRAME_TYPE; MAX_FRAME_LENGTH + 1]),
        Err(FrameError::InvalidLength { length }) if length == MAX_FRAME_LENGTH + 1
    ));
    assert!(matches!(
        decode_node_frame(b"\xff{}"),
        Err(FrameError::UnsupportedFrameType { frame_type: 0xff })
    ));
    assert!(matches!(
        decode_node_frame(&[NODE_MESSAGE_FRAME_TYPE, b'{']),
        Err(FrameError::DecodeJson(_))
    ));
    let invalid = ControllerToNodeMessage::EnsureWorktree(EnsureWorktreeMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        request_id: None,
        operation_id: OperationId::new(""),
        execution_id: ExecutionId::new("execution-create"),
        payload: EnsureWorktree { spec: spec() },
    });
    let mut body = vec![NODE_MESSAGE_FRAME_TYPE];
    body.extend(serde_json::to_vec(&invalid)?);
    assert!(matches!(
        decode_controller_frame(&body),
        Err(FrameError::InvalidMessage(_))
    ));
    let oversized = ControllerToNodeMessage::Hello(HelloMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Hello {
            controller_id: ControllerId::new("x".repeat(MAX_FRAME_LENGTH)),
            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
        },
    });
    assert!(matches!(
        encode_controller_frame(&oversized),
        Err(FrameError::InvalidLength { .. })
    ));
    Ok(())
}
