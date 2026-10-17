use super::session::node;
use super::support::*;
use ora_node_protocol::*;
use serde_json::{Value, json};

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// An install of one universal and one per-target plugin, with independent JSON.
fn install() -> Case {
    let download = |url: &str| PluginDownload {
        url: url.to_owned(),
        sha256: Sha256Digest::new(DIGEST),
    };
    Case {
        message: Message::Controller(ControllerToNodeMessage::InstallPlugins(
            InstallPluginsMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new("operation-1"),
                execution_id: ExecutionId::new("execution-plugins"),
                payload: InstallPlugins {
                    spec: InstallPluginsSpec {
                        node_id: NodeId::new("node-1"),
                        plugins: vec![
                            PluginInstall {
                                plugin_id: PluginId::new("ora.1a2b/claude-code"),
                                version: PluginVersion::new("1.2.0"),
                                release: PluginRelease::Universal {
                                    download: download("https://example.com/claude-code.tgz"),
                                },
                                agent_configuration: None,
                            },
                            PluginInstall {
                                plugin_id: PluginId::new("ora.1a2b/image-tools"),
                                version: PluginVersion::new("0.3.1"),
                                release: PluginRelease::Targets {
                                    targets: vec![PluginTargetDownload {
                                        target: "x86_64-unknown-linux-gnu".to_owned(),
                                        download: download("https://example.com/image-tools.tgz"),
                                    }],
                                },
                                agent_configuration: None,
                            },
                        ],
                    },
                },
            },
        )),
        wire: json!({
            "message_type": "install_plugins",
            "protocol_version": 1,
            "operation_id": "operation-1",
            "execution_id": "execution-plugins",
            "payload": {"spec": {"node_id": "node-1", "plugins": [
                {"plugin_id": "ora.1a2b/claude-code", "version": "1.2.0",
                 "release": {"kind": "universal", "download":
                    {"url": "https://example.com/claude-code.tgz", "sha256": DIGEST}}},
                {"plugin_id": "ora.1a2b/image-tools", "version": "0.3.1",
                 "release": {"kind": "targets", "targets": [{"target": "x86_64-unknown-linux-gnu",
                    "download": {"url": "https://example.com/image-tools.tgz", "sha256": DIGEST}}]}}
            ]}}
        }),
    }
}

/// Plugin results cover per-item outcomes and an execution-level failure.
fn results() -> Vec<(PluginExecutionResult, Value)> {
    let completed = PluginExecutionResult::PluginsCompleted(PluginsCompleted {
        node: node(),
        items: vec![
            PluginItemResult {
                plugin_id: PluginId::new("ora.1a2b/claude-code"),
                outcome: PluginItemOutcome::Installed {
                    version: PluginVersion::new("1.2.0"),
                },
            },
            PluginItemResult {
                plugin_id: PluginId::new("ora.1a2b/old-tool"),
                outcome: PluginItemOutcome::Removed {},
            },
            PluginItemResult {
                plugin_id: PluginId::new("ora.1a2b/image-tools"),
                outcome: PluginItemOutcome::Failed {
                    failure: PluginFailureCode::NoMatchingTarget,
                },
            },
        ],
    });
    let completed_wire = json!({"kind": "plugins_completed", "result": {
        "node": {"node_id": "node-1", "incarnation_id": "incarnation-1"},
        "items": [
            {"plugin_id": "ora.1a2b/claude-code", "outcome": {"kind": "installed", "version": "1.2.0"}},
            {"plugin_id": "ora.1a2b/old-tool", "outcome": {"kind": "removed"}},
            {"plugin_id": "ora.1a2b/image-tools", "outcome": {"kind": "failed", "failure": "no_matching_target"}}
        ]
    }});
    let failed = PluginExecutionResult::PluginsFailed(PluginsFailed {
        node: node(),
        failure: PluginsFailureCode::PluginRootUnavailable,
    });
    let failed_wire = json!({"kind": "plugins_failed", "result": {
        "node": {"node_id": "node-1", "incarnation_id": "incarnation-1"},
        "failure": "plugin_root_unavailable"
    }});
    vec![(completed, completed_wire), (failed, failed_wire)]
}

/// Delivers one result as the retained event and as a Completed status answer.
fn result_cases(result: PluginExecutionResult, wire: Value) -> [Case; 2] {
    [
        Case {
            message: Message::Node(NodeToControllerMessage::PluginsResult(
                PluginsResultMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new("operation-1"),
                    execution_id: ExecutionId::new("execution-plugins"),
                    sequence: Sequence::new(/*value*/ 1),
                    payload: result.clone(),
                },
            )),
            wire: json!({"message_type": "plugins_result", "protocol_version": 1,
                "operation_id": "operation-1", "execution_id": "execution-plugins",
                "sequence": 1, "payload": wire}),
        },
        Case {
            message: Message::Node(NodeToControllerMessage::ExecutionStatus(
                ExecutionStatusMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    operation_id: OperationId::new("operation-1"),
                    execution_id: ExecutionId::new("execution-plugins"),
                    payload: ExecutionStatus {
                        node: node(),
                        state: ExecutionState::Completed(ExecutionResult::Plugin(result)),
                    },
                },
            )),
            wire: json!({"message_type": "execution_status", "protocol_version": 1,
                "operation_id": "operation-1", "execution_id": "execution-plugins",
                "payload": {"node": {"node_id": "node-1", "incarnation_id": "incarnation-1"},
                    "state": {"state": "completed", "result": wire}}}),
        },
    ]
}

/// An install that also freezes an Agent declaration association on the universal plugin.
fn install_with_agent_configuration() -> Case {
    let mut case = install();
    let Message::Controller(ControllerToNodeMessage::InstallPlugins(message)) = &mut case.message
    else {
        unreachable!("install() builds an install command");
    };
    message.payload.spec.plugins[0].agent_configuration = Some(PluginAgentConfiguration {
        schema_version: 2,
        declaration_sha256: Sha256Digest::new(DIGEST),
    });
    let wire = json!({
        "message_type": "install_plugins",
        "protocol_version": 1,
        "operation_id": "operation-1",
        "execution_id": "execution-plugins",
        "payload": {"spec": {"node_id": "node-1", "plugins": [
            {"plugin_id": "ora.1a2b/claude-code", "version": "1.2.0",
             "agent_configuration": {"schema_version": 2, "declaration_sha256": DIGEST},
             "release": {"kind": "universal", "download":
                {"url": "https://example.com/claude-code.tgz", "sha256": DIGEST}}},
            {"plugin_id": "ora.1a2b/image-tools", "version": "0.3.1",
             "release": {"kind": "targets", "targets": [{"target": "x86_64-unknown-linux-gnu",
                "download": {"url": "https://example.com/image-tools.tgz", "sha256": DIGEST}}]}}
        ]}}
    });
    Case {
        message: case.message,
        wire,
    }
}

/// The install command has a fixed wire shape and the shared envelope guarantees.
#[tokio::test]
async fn install_plugins_round_trips_with_both_release_shapes() -> Result<(), TestError> {
    let case = install();
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    case.assert_envelope_rejections().await?;
    case.assert_fields(
        &[
            "/operation_id",
            "/execution_id",
            "/payload/spec/node_id",
            "/payload/spec/plugins",
        ],
        &[
            ("/operation_id", "operation_id"),
            ("/execution_id", "execution_id"),
            ("/payload/spec/node_id", "node_id"),
        ],
    )
    .await
}

/// A plan may freeze the declaration association one catalog entry declared.
#[tokio::test]
async fn install_plugins_round_trips_an_agent_declaration_association() -> Result<(), TestError> {
    let case = install_with_agent_configuration();
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    Ok(())
}

/// Rejects declaration associations a Node could not prove against any package.
#[tokio::test]
async fn install_plugins_rejects_unprovable_agent_declarations() -> Result<(), TestError> {
    let base = install_with_agent_configuration().wire;
    let cases: [(&str, Value, MessageValidationError); 2] = [
        (
            "/payload/spec/plugins/0/agent_configuration/schema_version",
            json!(0),
            MessageValidationError::EmptyField {
                field: "agent_configuration.schema_version",
            },
        ),
        (
            "/payload/spec/plugins/0/agent_configuration/declaration_sha256",
            json!(DIGEST.to_uppercase()),
            MessageValidationError::InvalidSha256,
        ),
    ];
    for (path, value, expected) in cases {
        let mut wire = base.clone();
        replace(&mut wire, path, value);
        reject_semantics(Peer::Controller, &wire, path, expected).await?;
    }
    Ok(())
}

/// Rejects payloads a Node could not verify before download, in both directions of the codec.
#[tokio::test]
async fn install_plugins_rejects_unverifiable_payloads() -> Result<(), TestError> {
    let base = install().wire;
    let cases: [(&str, Value, MessageValidationError); 7] = [
        (
            "/payload/spec/plugins",
            json!([]),
            MessageValidationError::EmptyPluginSet,
        ),
        (
            "/payload/spec/plugins/1/plugin_id",
            json!("ora.1a2b/claude-code"),
            MessageValidationError::DuplicatePlugin,
        ),
        (
            "/payload/spec/plugins/0/plugin_id",
            json!("claude-code"),
            MessageValidationError::InvalidPluginId,
        ),
        (
            "/payload/spec/plugins/0/release/download/sha256",
            json!(DIGEST.to_uppercase()),
            MessageValidationError::InvalidSha256,
        ),
        (
            "/payload/spec/plugins/0/release/download/url",
            json!("file:///tmp/claude-code.tgz"),
            MessageValidationError::InvalidPluginRelease,
        ),
        (
            "/payload/spec/plugins/1/release/targets",
            json!([]),
            MessageValidationError::InvalidPluginRelease,
        ),
        (
            "/payload/spec/plugins/0/version",
            json!(" "),
            MessageValidationError::EmptyField { field: "version" },
        ),
    ];
    for (path, value, expected) in cases {
        let mut wire = base.clone();
        replace(&mut wire, path, value);
        reject_semantics(Peer::Controller, &wire, path, expected).await?;
    }
    Ok(())
}

/// A removal names each plugin once, without download data.
#[tokio::test]
async fn remove_plugins_round_trips() -> Result<(), TestError> {
    let case = Case {
        message: Message::Controller(ControllerToNodeMessage::RemovePlugins(
            RemovePluginsMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: OperationId::new("operation-2"),
                execution_id: ExecutionId::new("execution-remove"),
                payload: RemovePlugins {
                    spec: RemovePluginsSpec {
                        node_id: NodeId::new("node-1"),
                        plugins: vec![PluginRemoval {
                            plugin_id: PluginId::new("ora.1a2b/old-tool"),
                            version: PluginVersion::new("2.0.0"),
                        }],
                    },
                },
            },
        )),
        wire: json!({"message_type": "remove_plugins", "protocol_version": 1,
            "operation_id": "operation-2", "execution_id": "execution-remove",
            "payload": {"spec": {"node_id": "node-1", "plugins": [
                {"plugin_id": "ora.1a2b/old-tool", "version": "2.0.0"}]}}}),
    };
    case.assert_wire().await?;
    case.assert_round_trip().await?;
    case.assert_envelope_rejections().await?;
    let mut wire = case.wire.clone();
    replace(&mut wire, "/payload/spec/plugins", json!([]));
    reject_semantics(
        Peer::Controller,
        &wire,
        "empty removal",
        MessageValidationError::EmptyPluginSet,
    )
    .await
}

/// Plugin results decode as the plugin family both as events and inside untagged status results.
#[tokio::test]
async fn plugin_results_round_trip_as_events_and_completed_status() -> Result<(), TestError> {
    for (result, wire) in results() {
        let [event, status] = result_cases(result, wire);
        for case in [&event, &status] {
            case.assert_wire().await?;
            case.assert_round_trip().await?;
            case.assert_envelope_rejections().await?;
        }
        status.assert_historical_node().await?;
    }
    let (result, wire) = results().remove(0);
    let [event, _] = result_cases(result, wire);
    let mut wire = event.wire;
    replace(
        &mut wire,
        "/payload/result/items/1/plugin_id",
        json!("ora.1a2b/claude-code"),
    );
    reject_semantics(
        Peer::Node,
        &wire,
        "duplicate item",
        MessageValidationError::DuplicatePlugin,
    )
    .await
}
