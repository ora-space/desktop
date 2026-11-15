use super::support::*;
use ora_node_protocol::*;
use serde_json::json;

#[path = "session/fixtures.rs"]
mod fixtures;

/// A clone-only Node is valid without claiming an unimplemented Worktree capability.
#[tokio::test]
async fn accepts_independent_and_combined_execution_capabilities() -> Result<(), TestError> {
    for (capabilities, wire) in [
        (
            vec![NodeCapability::RepositoryClone],
            json!(["repository_clone"]),
        ),
        (
            vec![
                NodeCapability::RepositoryClone,
                NodeCapability::WorktreeExecution,
            ],
            json!(["repository_clone", "worktree_execution"]),
        ),
        (
            vec![
                NodeCapability::RepositoryClone,
                NodeCapability::PluginInstall,
                NodeCapability::AgentSession,
                NodeCapability::SessionMcp,
                NodeCapability::RevisionDelivery,
                NodeCapability::RevisionRestore,
            ],
            json!([
                "repository_clone",
                "plugin_install",
                "agent_session",
                "session_mcp",
                "revision_delivery",
                "revision_restore"
            ]),
        ),
    ] {
        let mut case = fixtures::hello_accepted();
        let Message::Node(NodeToControllerMessage::HelloAccepted(message)) = &mut case.message
        else {
            panic!("expected handshake")
        };
        message.payload.capabilities = capabilities;
        case.wire["payload"]["capabilities"] = wire;
        case.assert_wire().await?;
        case.assert_round_trip().await?;
    }
    let mut wire = fixtures::hello_accepted().wire;
    wire["payload"]["capabilities"] = json!(["repository_clone", "repository_clone"]);
    reject_semantics(
        Peer::Node,
        &wire,
        "duplicate clone capability",
        MessageValidationError::DuplicateCapability,
    )
    .await?;
    wire["payload"]["capabilities"] = json!(["unknown_execution"]);
    reject_structure(Peer::Node, &wire, "unknown capability").await;
    Ok(())
}

/// Returns the Node identity attached to every result fixture.
pub(super) fn node() -> NodeRuntimeIdentity {
    NodeRuntimeIdentity {
        node_id: NodeId::new("node-1"),
        incarnation_id: NodeIncarnationId::new("incarnation-1"),
    }
}

/// Rejects inconsistent handshake declarations through both public codec paths.
#[tokio::test]
async fn rejects_inconsistent_handshakes() -> Result<(), TestError> {
    for (case, path, value, expected) in [
        (
            fixtures::hello(),
            "/payload/supported_versions",
            json!([]),
            MessageValidationError::NoSupportedProtocolVersions,
        ),
        (
            fixtures::hello(),
            "/payload/supported_versions",
            json!([1, 1]),
            MessageValidationError::DuplicateProtocolVersion { version: 1 },
        ),
        (
            fixtures::hello(),
            "/payload/supported_versions",
            json!([2]),
            MessageValidationError::EnvelopeVersionNotAdvertised { version: 1 },
        ),
        (
            fixtures::hello_accepted(),
            "/payload/selected_version",
            json!(2),
            MessageValidationError::SelectedVersionMismatch {
                selected: 2,
                envelope: 1,
            },
        ),
        (
            fixtures::hello_accepted(),
            "/payload/capabilities",
            json!([]),
            MessageValidationError::NoExecutionCapabilities,
        ),
        (
            fixtures::hello_accepted(),
            "/payload/capabilities",
            json!(["worktree_execution", "worktree_execution"]),
            MessageValidationError::DuplicateCapability,
        ),
    ] {
        let peer = case.peer();
        let mut wire = case.wire;
        receive(peer, &wire).await?;
        replace(&mut wire, path, value);
        reject_semantics(peer, &wire, path, expected).await?;
    }
    Ok(())
}

/// Proves fragmented I/O preserves every legal message in this business slice.
#[tokio::test]
async fn round_trips_messages() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_round_trip().await?;
    }
    Ok(())
}

/// Locks independent wire shapes, optional output fields, and extension acceptance.
#[tokio::test]
async fn preserves_wire_shapes() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_wire().await?;
        case.assert_extensions().await?;
    }
    Ok(())
}

/// Applies version and direction constraints to every legal shape in this slice.
#[tokio::test]
async fn rejects_versions_directions_and_payloads() -> Result<(), TestError> {
    for case in fixtures::cases() {
        case.assert_envelope_rejections().await?;
    }
    Ok(())
}

/// Checks required structure separately from present-but-empty opaque values.
#[tokio::test]
async fn rejects_missing_and_empty_fields() -> Result<(), TestError> {
    fixtures::hello()
        .assert_fields(
            &[
                "/message_type",
                "/protocol_version",
                "/payload",
                "/payload/supported_versions",
                "/payload/controller_id",
            ],
            &[("/payload/controller_id", "controller_id")],
        )
        .await?;
    fixtures::hello_accepted()
        .assert_fields(
            &[
                "/message_type",
                "/protocol_version",
                "/payload",
                "/payload/selected_version",
                "/payload/capabilities",
                "/payload/node/node_id",
                "/payload/node/incarnation_id",
            ],
            &[
                ("/payload/node/node_id", "node.node_id"),
                ("/payload/node/incarnation_id", "node.incarnation_id"),
            ],
        )
        .await?;
    fixtures::heartbeat()
        .assert_fields(
            &[
                "/message_type",
                "/protocol_version",
                "/payload",
                "/payload/node/node_id",
                "/payload/node/incarnation_id",
            ],
            &[
                ("/payload/node/node_id", "node.node_id"),
                ("/payload/node/incarnation_id", "node.incarnation_id"),
            ],
        )
        .await?;
    fixtures::controller_heartbeat()
        .assert_fields(
            &[
                "/message_type",
                "/protocol_version",
                "/payload",
                "/payload/controller_id",
            ],
            &[("/payload/controller_id", "controller_id")],
        )
        .await?;
    Ok(())
}

/// Adapter facts travel only beside `model_proxy`, with a known contract and distinct protocols.
#[tokio::test]
async fn model_adapter_capability_round_trips_and_requires_model_proxy() -> Result<(), TestError> {
    let mut case = fixtures::hello_accepted();
    let Message::Node(NodeToControllerMessage::HelloAccepted(accepted)) = &mut case.message else {
        panic!("fixture must be a hello_accepted")
    };
    accepted.payload.capabilities = vec![NodeCapability::AgentSession, NodeCapability::ModelProxy];
    accepted.payload.model_adapter = Some(ModelAdapterCapability {
        contract_version: MODEL_ADAPTER_CONTRACT_VERSION,
        target: "x86_64-unknown-linux-gnu".into(),
        protocols: vec![
            ModelProtocol::OpenaiCompletions,
            ModelProtocol::AnthropicMessages,
        ],
    });
    case.wire["payload"]["capabilities"] = json!(["agent_session", "model_proxy"]);
    case.wire["payload"]["model_adapter"] = json!({
        "contract_version": 1,
        "target": "x86_64-unknown-linux-gnu",
        "protocols": ["openai-completions", "anthropic-messages"],
    });
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    for (path, value) in [
        ("/payload/capabilities", json!(["agent_session"])),
        ("/payload/model_adapter/contract_version", json!(2)),
        ("/payload/model_adapter/target", json!("")),
        ("/payload/model_adapter/protocols", json!([])),
        (
            "/payload/model_adapter/protocols",
            json!(["anthropic-messages", "anthropic-messages"]),
        ),
    ] {
        let mut wire = case.wire.clone();
        replace(&mut wire, path, value);
        reject_semantics(
            Peer::Node,
            &wire,
            path,
            MessageValidationError::InvalidModelAdapterCapability,
        )
        .await?;
    }
    let mut wire = case.wire.clone();
    replace(
        &mut wire,
        "/payload/model_adapter/protocols",
        json!(["openai-responses"]),
    );
    reject_structure(Peer::Node, &wire, "unknown model protocol").await;
    Ok(())
}
