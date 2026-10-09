//! Real service/actor composition, command ordering, shared leases and process restart behavior.
use super::*;
use pretty_assertions::assert_eq;
#[path = "agent_sessions/authority.rs"]
mod authority;
#[path = "agent_sessions/delivery.rs"]
mod delivery;
#[path = "agent_sessions/recovery.rs"]
mod recovery;
#[path = "agent_sessions/support.rs"]
mod support;
#[path = "agent_sessions/window.rs"]
mod window;
use support::*;

/// The production service runs the installed Agent, deduplicates commands, and shares use leases.
#[test]
fn session_commands_use_the_production_ledger_and_installer_catalog() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        let mut records = Vec::new();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(start("hello")),
                )
                .await;
                loop {
                    match receive(&mut stream).await {
                        NodeToControllerMessage::ThreadEvent(event) => {
                            let done = event
                                .payload
                                .record
                                .get("type")
                                .and_then(serde_json::Value::as_str)
                                == Some("turnEnded");
                            records.push(event);
                            if done {
                                break;
                            }
                        }
                        NodeToControllerMessage::ExecutionStatus(_) => {}
                        other => panic!("unexpected {other:?}"),
                    }
                }
                send(
                    &mut stream,
                    ControllerToNodeMessage::RemovePlugins(RemovePluginsMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        operation_id: OperationId::new("remove-op"),
                        execution_id: ExecutionId::new("remove-exec"),
                        payload: RemovePlugins {
                            spec: RemovePluginsSpec {
                                node_id: NodeId::new("test-node"),
                                plugins: vec![PluginRemoval {
                                    plugin_id: start("").payload.spec.agent_plugin_id,
                                    version: PluginVersion::new("1.0.0"),
                                }],
                            },
                        },
                    }),
                )
                .await;
                loop {
                    match receive(&mut stream).await {
                        NodeToControllerMessage::PluginsResult(result) => {
                            assert!(
                                serde_json::to_string(&result)
                                    .unwrap()
                                    .contains("plugin_in_use")
                            );
                            break;
                        }
                        NodeToControllerMessage::ExecutionStatus(_) => {}
                        other => panic!("unexpected {other:?}"),
                    }
                }
                send(&mut stream, turn()).await;
                send(&mut stream, turn()).await;
                let mut accepted = 0;
                let mut finished = false;
                while accepted < 2 || !finished {
                    match receive(&mut stream).await {
                        NodeToControllerMessage::SessionCommandAccepted(reply) => {
                            assert_eq!(reply.payload.command_id, CommandId::new("second-command"));
                            accepted += 1;
                        }
                        NodeToControllerMessage::ThreadEvent(event) => {
                            finished |= event
                                .payload
                                .record
                                .get("type")
                                .and_then(serde_json::Value::as_str)
                                == Some("turnEnded");
                            records.push(event);
                        }
                        other => panic!("unexpected {other:?}"),
                    }
                }
                send(&mut stream, end()).await;
                let mut accepted_end = false;
                loop {
                    match receive(&mut stream).await {
                        NodeToControllerMessage::SessionCommandAccepted(_) => accepted_end = true,
                        NodeToControllerMessage::ThreadEvent(event) => records.push(event),
                        NodeToControllerMessage::AgentSessionEnded(event) => {
                            assert!(accepted_end);
                            let AgentSessionResult::AgentSessionEnded(result) = event.payload;
                            assert_eq!(result.reason, AgentSessionEndReason::UserEnded);
                            assert_eq!(event.sequence.value(), records.len() as u64 + 1);
                            break;
                        }
                        other => panic!("unexpected {other:?}"),
                    }
                }
                send(&mut stream, turn()).await;
                assert!(matches!(
                    receive(&mut stream).await,
                    NodeToControllerMessage::SessionCommandRejected(_)
                ));
            });
        child.terminate();
        assert_eq!(
            records
                .iter()
                .map(|event| serde_json::Value::Object(event.payload.record.clone()))
                .collect::<Vec<_>>(),
            history(&fixture)
        );
        assert_eq!(
            records
                .iter()
                .map(|event| event.sequence.value())
                .collect::<Vec<_>>(),
            (1..=records.len() as u64).collect::<Vec<_>>()
        );
        assert_eq!(
            fs::read_to_string(root.join("echo-agent.pids"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|e| e.payload.turn_id == Some(TurnId::new("second"))
                    && e.payload
                        .record
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        == Some("turnEnded"))
                .count(),
            1
        );
    });
}
