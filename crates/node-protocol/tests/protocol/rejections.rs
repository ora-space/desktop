use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
enum Peer {
    Controller,
    Node,
}

/// Supplies every envelope and nested result shape to the shared rejection matrix.
fn messages() -> Result<Vec<(Peer, Value)>, serde_json::Error> {
    let mut messages = vec![
        (
            Peer::Controller,
            json!({"message_type":"hello", "protocol_version":1,
            "payload":{"controller_id":"controller-1", "supported_versions":[1]}}),
        ),
        (
            Peer::Node,
            json!({"message_type":"hello_accepted", "protocol_version":1,
            "payload":{"selected_version":1, "node":node(), "capabilities":["worktree_execution"]}}),
        ),
        (
            Peer::Node,
            json!({"message_type":"heartbeat", "protocol_version":1,
            "payload":{"node":node()}}),
        ),
    ];
    for kind in [
        "ensure_worktree",
        "remove_worktree",
        "get_execution_status",
        "event_ack",
    ] {
        let payload = match kind {
            "ensure_worktree" | "remove_worktree" => json!({"spec":spec()}),
            _ => json!({"node_id":"node-1"}),
        };
        let mut wire = json!({"message_type":kind, "protocol_version":1,
            "operation_id":"operation-1", "execution_id":"execution-1", "payload":payload});
        if kind == "event_ack" {
            wire["sequence"] = json!(7);
        }
        if matches!(kind, "ensure_worktree" | "remove_worktree") {
            wire["request_id"] = json!("request-1");
        }
        messages.push((Peer::Controller, wire));
    }
    for (kind, result) in [
        (
            "worktree_ready",
            WorktreeExecutionResult::Ready(ready_result()),
        ),
        (
            "worktree_failed",
            WorktreeExecutionResult::Failed(failed_result()),
        ),
        (
            "worktree_removed",
            WorktreeExecutionResult::Removed(removed_result()),
        ),
        (
            "worktree_removal_failed",
            WorktreeExecutionResult::RemovalFailed(removal_failed_result()),
        ),
    ] {
        let result = serde_json::to_value(result)?;
        messages.push((
            Peer::Node,
            json!({"message_type":kind, "protocol_version":1,
            "request_id":"request-1", "operation_id":"operation-1", "execution_id":"execution-1",
            "sequence":7, "payload":result["result"]}),
        ));
        messages.push((
            Peer::Node,
            json!({"message_type":"execution_status", "protocol_version":1,
            "operation_id":"operation-1", "execution_id":"execution-1",
            "payload":{"node":node(), "state":{"state":"completed", "result":result}}}),
        ));
    }
    for state in ["unknown", "accepted", "running"] {
        messages.push((
            Peer::Node,
            json!({"message_type":"execution_status", "protocol_version":1,
            "operation_id":"operation-1", "execution_id":"execution-1",
            "payload":{"node":node(), "state":{"state":state}}}),
        ));
    }
    Ok(messages)
}

/// Uses raw wire input to exercise receiving independently of sending validation.
async fn receive(peer: Peer, wire: &Value) -> Result<(), TestError> {
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
async fn reject_semantics(
    peer: Peer,
    wire: &Value,
    expected: MessageValidationError,
) -> Result<(), TestError> {
    match receive(peer, wire).await {
        Err(TestError::Frame(FrameError::InvalidMessage(error))) => assert_eq!(error, expected),
        other => panic!("expected {expected:?} for {wire}, got {other:?}"),
    }
    let mut output = Vec::new();
    let sent = match peer {
        Peer::Controller => {
            write_controller_message(&mut output, &serde_json::from_value(wire.clone())?).await
        }
        Peer::Node => write_node_message(&mut output, &serde_json::from_value(wire.clone())?).await,
    };
    match sent {
        Err(FrameError::InvalidMessage(error)) => assert_eq!(error, expected),
        other => panic!("expected outbound {expected:?}, got {other:?}"),
    }
    assert_eq!(output, Vec::<u8>::new());
    Ok(())
}

/// Enumerates opaque fields while excluding enum discriminants that serde validates structurally.
fn opaque_paths(value: &Value, prefix: &str, paths: &mut Vec<String>) {
    if let Value::Object(fields) = value {
        for (key, value) in fields {
            let path = format!("{prefix}/{key}");
            if value.is_string()
                && !matches!(
                    key.as_str(),
                    "message_type" | "state" | "kind" | "code" | "outcome"
                )
            {
                paths.push(path);
            } else {
                opaque_paths(value, &path, paths);
            }
        }
    }
}

/// Tests missing, empty and whitespace identities and opaque domain fields in every message shape.
#[tokio::test]
async fn rejects_missing_and_empty_fields() -> Result<(), TestError> {
    for (peer, valid) in messages()? {
        receive(peer, &valid).await?;
        let mut paths = Vec::new();
        opaque_paths(&valid, "", &mut paths);
        for path in paths {
            for empty in ["", " \t\n"] {
                let mut wire = valid.clone();
                *wire
                    .pointer_mut(&path)
                    .ok_or_else(|| io::Error::other("fixture field missing"))? = json!(empty);
                let field = if path.contains("/node/") {
                    if path.ends_with("/node_id") {
                        "node.node_id"
                    } else {
                        "node.incarnation_id"
                    }
                } else if path.contains("/main_workspace/") {
                    if path.ends_with("/path") {
                        "main_workspace.path"
                    } else {
                        "main_workspace.workspace_id"
                    }
                } else {
                    match path
                        .rsplit('/')
                        .next()
                        .ok_or_else(|| io::Error::other("field name missing"))?
                    {
                        "controller_id" => "controller_id",
                        "request_id" => "request_id",
                        "operation_id" => "operation_id",
                        "execution_id" => "execution_id",
                        "node_id" => "node_id",
                        "workspace_id" => "workspace_id",
                        "worktree_id" => "worktree_id",
                        "repository" => "repository",
                        "base_ref" => "base_ref",
                        "expected_branch" => "expected_branch",
                        "directory_name" => "path_policy.directory_name",
                        "path" => "facts.path",
                        "branch" => "facts.branch",
                        "base_commit" => "facts.base_commit",
                        "message" => "failure.message",
                        other => panic!("unmapped field {other}"),
                    }
                };
                reject_semantics(peer, &wire, MessageValidationError::EmptyField { field }).await?;
            }
            let mut wire = valid.clone();
            let (parent, key) = path
                .rsplit_once('/')
                .ok_or_else(|| io::Error::other("invalid field path"))?;
            wire.pointer_mut(parent)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| io::Error::other("fixture object missing"))?
                .remove(key);
            if key == "request_id" {
                receive(peer, &wire).await?;
            } else {
                assert!(
                    matches!(
                        receive(peer, &wire).await,
                        Err(TestError::Frame(FrameError::DecodeJson(_)))
                    ),
                    "{path}: {wire}"
                );
            }
        }
    }
    Ok(())
}

/// Provides a direct counterexample for each handshake rule and every envelope version check.
#[tokio::test]
async fn rejects_inconsistent_handshakes_and_versions() -> Result<(), TestError> {
    for (peer, mut wire) in messages()? {
        wire["protocol_version"] = json!(2);
        reject_semantics(
            peer,
            &wire,
            MessageValidationError::UnsupportedProtocolVersion {
                actual: 2,
                expected: 1,
            },
        )
        .await?;
    }
    let fixtures = messages()?;
    for (index, path, value, expected) in [
        (
            0,
            "/payload/supported_versions",
            json!([]),
            MessageValidationError::NoSupportedProtocolVersions,
        ),
        (
            0,
            "/payload/supported_versions",
            json!([1, 1]),
            MessageValidationError::DuplicateProtocolVersion { version: 1 },
        ),
        (
            0,
            "/payload/supported_versions",
            json!([2]),
            MessageValidationError::EnvelopeVersionNotAdvertised { version: 1 },
        ),
        (
            1,
            "/payload/selected_version",
            json!(2),
            MessageValidationError::SelectedVersionMismatch {
                selected: 2,
                envelope: 1,
            },
        ),
        (
            1,
            "/payload/capabilities",
            json!([]),
            MessageValidationError::WorktreeCapabilityMissing,
        ),
        (
            1,
            "/payload/capabilities",
            json!(["worktree_execution", "worktree_execution"]),
            MessageValidationError::DuplicateCapability,
        ),
    ] {
        let (peer, mut wire) = fixtures[index].clone();
        *wire
            .pointer_mut(path)
            .ok_or_else(|| io::Error::other("fixture field missing"))? = value;
        reject_semantics(peer, &wire, expected).await?;
    }
    Ok(())
}

/// Refuses wrong directions, mismatched payloads, missing envelope fields and invalid state shapes.
#[tokio::test]
async fn rejects_structural_contradictions() -> Result<(), TestError> {
    for (peer, valid) in messages()? {
        let opposite = match peer {
            Peer::Controller => Peer::Node,
            Peer::Node => Peer::Controller,
        };
        assert!(matches!(
            receive(opposite, &valid).await,
            Err(TestError::Frame(FrameError::DecodeJson(_)))
        ));
        for field in ["message_type", "protocol_version", "payload", "sequence"] {
            let mut wire = valid.clone();
            if wire
                .as_object_mut()
                .ok_or_else(|| io::Error::other("fixture envelope missing"))?
                .remove(field)
                .is_some()
            {
                assert!(
                    matches!(
                        receive(peer, &wire).await,
                        Err(TestError::Frame(FrameError::DecodeJson(_)))
                    ),
                    "{wire}"
                );
            }
        }
        let mut mismatch = valid.clone();
        mismatch["payload"] = json!({"unrelated":true});
        assert!(matches!(
            receive(peer, &mismatch).await,
            Err(TestError::Frame(FrameError::DecodeJson(_)))
        ));
        if valid["message_type"] == "execution_status" {
            for state in [
                json!({"state":"completed"}),
                json!({"state":"unknown","result":{}}),
                json!({"state":"accepted","result":{}}),
                json!({"state":"running","result":{}}),
                json!({"state":"invalid"}),
                json!({"state":"completed","result":{"kind":"ready","result":{}}}),
            ] {
                let mut wire = valid.clone();
                wire["payload"]["state"] = state;
                assert!(
                    matches!(
                        receive(peer, &wire).await,
                        Err(TestError::Frame(FrameError::DecodeJson(_)))
                    ),
                    "{wire}"
                );
            }
        }
    }
    Ok(())
}

/// Rejects an oversized serialized message before writing its length header.
#[tokio::test]
async fn rejects_oversized_outbound_message_without_writing() -> Result<(), TestError> {
    let message = ControllerToNodeMessage::Hello {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Hello {
            controller_id: ControllerId::new("x".repeat(MAX_FRAME_LENGTH)),
            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
        },
    };
    let mut output = Vec::new();
    assert!(matches!(
        write_controller_message(&mut output, &message).await,
        Err(FrameError::InvalidLength { .. })
    ));
    assert_eq!(output, Vec::<u8>::new());
    Ok(())
}
