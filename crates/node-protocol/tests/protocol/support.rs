use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::io;
use thiserror::Error;
use tokio::io::duplex;

#[derive(Debug, Error)]
pub(super) enum TestError {
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

/// Forces a Controller message through a three-byte stream buffer to fragment its frame.
pub(super) async fn round_trip_controller(
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
pub(super) async fn round_trip_node(
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
pub(super) fn framed(frame_type: u8, payload: &[u8]) -> Result<Vec<u8>, std::num::TryFromIntError> {
    let length = u32::try_from(payload.len() + 1)?;
    let mut frame = Vec::with_capacity(payload.len() + 5);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.push(frame_type);
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Compares the preserved I/O category without relying on formatted error text.
pub(super) fn assert_io_kind(error: FrameError, expected: io::ErrorKind) {
    match error {
        FrameError::Io(source) => assert_eq!(source.kind(), expected),
        other => panic!("expected I/O error, got {other:?}"),
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Peer {
    Controller,
    Node,
}

/// Uses raw wire input to exercise receiving independently of sending validation.
pub(super) async fn receive(peer: Peer, wire: &Value) -> Result<(), TestError> {
    let bytes = framed(NODE_MESSAGE_FRAME_TYPE, &serde_json::to_vec(wire)?)?;
    match peer {
        Peer::Controller => {
            read_controller_message(&mut bytes.as_slice()).await?;
        }
        Peer::Node => {
            read_node_message(&mut bytes.as_slice()).await?;
        }
    }
    Ok(())
}

/// Verifies outbound semantic rejection occurs before any bytes reach the writer.
pub(super) async fn reject_semantics(
    peer: Peer,
    wire: &Value,
    context: &str,
    expected: MessageValidationError,
) -> Result<(), TestError> {
    match receive(peer, wire).await {
        Err(TestError::Frame(FrameError::InvalidMessage(error))) => {
            assert_eq!(error, expected, "{context}: {wire}")
        }
        other => panic!("{context}: expected {expected:?} for {wire}, got {other:?}"),
    }
    let mut output = Vec::new();
    let sent = match peer {
        Peer::Controller => {
            write_controller_message(&mut output, &serde_json::from_value(wire.clone())?).await
        }
        Peer::Node => write_node_message(&mut output, &serde_json::from_value(wire.clone())?).await,
    };
    match sent {
        Err(FrameError::InvalidMessage(error)) => assert_eq!(error, expected, "{context}: {wire}"),
        other => panic!("{context}: expected outbound {expected:?} for {wire}, got {other:?}"),
    }
    assert_eq!(output, Vec::<u8>::new());
    Ok(())
}

/// Keeps the public codec direction explicit in shared test mechanics.
pub(super) enum Message {
    Controller(ControllerToNodeMessage),
    Node(NodeToControllerMessage),
}

/// An independently authored typed message and its expected JSON object.
pub(super) struct Case {
    pub message: Message,
    pub wire: Value,
}

impl Case {
    /// Checks reporter identity independently of the business that owns a Completed payload.
    pub async fn assert_historical_node(mut self) -> Result<(), TestError> {
        let Message::Node(NodeToControllerMessage::ExecutionStatus(ExecutionStatusMessage {
            payload,
            ..
        })) = &mut self.message
        else {
            panic!("expected status fixture")
        };
        payload.node.incarnation_id = NodeIncarnationId::new("incarnation-2");
        replace(
            &mut self.wire,
            "/payload/node/incarnation_id",
            json!("incarnation-2"),
        );
        self.assert_wire().await?;
        self.assert_round_trip().await?;
        replace(&mut self.wire, "/payload/node/node_id", json!("node-2"));
        reject_semantics(
            self.peer(),
            &self.wire,
            "/payload/node/node_id",
            MessageValidationError::CompletedNodeMismatch {
                reporter: NodeId::new("node-2"),
                result: NodeId::new("node-1"),
            },
        )
        .await?;
        Ok(())
    }

    /// Verifies both public codec paths against independently authored expectations.
    pub async fn assert_wire(&self) -> Result<(), TestError> {
        let bytes = framed(NODE_MESSAGE_FRAME_TYPE, &serde_json::to_vec(&self.wire)?)?;
        let mut output = Vec::new();
        match &self.message {
            Message::Controller(expected) => {
                write_controller_message(&mut output, expected).await?;
                assert_eq!(
                    read_controller_message(&mut bytes.as_slice()).await?,
                    Some(expected.clone())
                );
            }
            Message::Node(expected) => {
                write_node_message(&mut output, expected).await?;
                assert_eq!(
                    read_node_message(&mut bytes.as_slice()).await?,
                    Some(expected.clone())
                );
            }
        }
        assert_eq!(serde_json::from_slice::<Value>(&output[5..])?, self.wire);
        Ok(())
    }
}

impl Case {
    /// Selects the public codec from the typed fixture, never from its JSON tag.
    pub fn peer(&self) -> Peer {
        match &self.message {
            Message::Controller(_) => Peer::Controller,
            Message::Node(_) => Peer::Node,
        }
    }

    /// Exercises fragmented I/O for the complete expected message.
    pub async fn assert_round_trip(&self) -> Result<(), TestError> {
        match &self.message {
            Message::Controller(expected) => {
                assert_eq!(round_trip_controller(expected.clone()).await?, *expected)
            }
            Message::Node(expected) => {
                assert_eq!(round_trip_node(expected.clone()).await?, *expected)
            }
        }
        Ok(())
    }

    /// Keeps extension fields accepted without changing the decoded message.
    pub async fn assert_extensions(&self) -> Result<(), TestError> {
        let mut wire = self.wire.clone();
        wire["future_envelope_field"] = json!({"value":1});
        wire["payload"]["future_payload_field"] = json!(true);
        let bytes = framed(NODE_MESSAGE_FRAME_TYPE, &serde_json::to_vec(&wire)?)?;
        match &self.message {
            Message::Controller(expected) => assert_eq!(
                read_controller_message(&mut bytes.as_slice()).await?,
                Some(expected.clone())
            ),
            Message::Node(expected) => assert_eq!(
                read_node_message(&mut bytes.as_slice()).await?,
                Some(expected.clone())
            ),
        }
        Ok(())
    }

    /// Applies shared envelope guarantees to a business-owned fixture.
    pub async fn assert_envelope_rejections(&self) -> Result<(), TestError> {
        let peer = self.peer();
        receive(peer, &self.wire).await?;
        let opposite = match peer {
            Peer::Controller => Peer::Node,
            Peer::Node => Peer::Controller,
        };
        reject_structure(opposite, &self.wire, "opposite direction").await;
        let mut wire = self.wire.clone();
        replace(&mut wire, "/protocol_version", json!(2));
        reject_semantics(
            peer,
            &wire,
            "/protocol_version",
            MessageValidationError::UnsupportedProtocolVersion {
                actual: 2,
                expected: 1,
            },
        )
        .await?;
        let mut wire = self.wire.clone();
        replace(&mut wire, "/payload", json!({"unrelated":true}));
        reject_structure(peer, &wire, "/payload").await;
        Ok(())
    }

    /// Mutates exactly one explicitly listed field after proving the baseline legal.
    pub async fn assert_fields(
        &self,
        required: &[&str],
        opaque: &[(&str, &'static str)],
    ) -> Result<(), TestError> {
        let peer = self.peer();
        receive(peer, &self.wire).await?;
        for path in required {
            let mut wire = self.wire.clone();
            let (parent, key) = path
                .rsplit_once('/')
                .ok_or_else(|| io::Error::other("invalid JSON pointer"))?;
            assert!(
                wire.pointer_mut(parent)
                    .and_then(Value::as_object_mut)
                    .ok_or_else(|| io::Error::other("missing fixture object"))?
                    .remove(key)
                    .is_some(),
                "missing {path} in {}",
                self.wire
            );
            reject_structure(peer, &wire, path).await;
        }
        for (path, field) in opaque {
            for empty in ["", " \t\n"] {
                let mut wire = self.wire.clone();
                replace(&mut wire, path, json!(empty));
                reject_semantics(
                    peer,
                    &wire,
                    path,
                    MessageValidationError::EmptyField { field },
                )
                .await?;
            }
        }
        // Pad every opaque value together so equal NodeIds still agree in Completed results.
        let mut wire = self.wire.clone();
        for (path, _) in opaque {
            let original = wire
                .pointer(path)
                .and_then(Value::as_str)
                .ok_or_else(|| io::Error::other("missing opaque fixture string"))?;
            let padded = format!(" {original}\t");
            replace(&mut wire, path, json!(padded));
        }
        let message = match peer {
            Peer::Controller => Message::Controller(serde_json::from_value(wire.clone())?),
            Peer::Node => Message::Node(serde_json::from_value(wire.clone())?),
        };
        Case { message, wire }.assert_wire().await?;
        Ok(())
    }
}

/// Fails immediately if a counterexample points at a field absent from its baseline.
pub(super) fn replace(wire: &mut Value, path: &str, value: Value) {
    *wire
        .pointer_mut(path)
        .unwrap_or_else(|| panic!("fixture field {path} missing")) = value;
}

/// Keeps structural failures distinct from semantic validation failures.
pub(super) async fn reject_structure(peer: Peer, wire: &Value, context: &str) {
    assert!(
        matches!(
            receive(peer, wire).await,
            Err(TestError::Frame(FrameError::DecodeJson(_)))
        ),
        "{context}: expected DecodeJson for {wire}"
    );
}
