//! The `turn_cancel` handshake and the takeover boundary of sessions that chose a keep behaviour.
//!
//! Evidence for specs/test-cases/node/protocol/cancel-turn-and-keep-session.md cases 16 and 17.
use super::*;
use pretty_assertions::assert_eq;

/// The reason the Node logs when it refuses a takeover; it closes the stream without a frame.
const REFUSAL: &str = "sessions that chose turn_cancel need a Controller that negotiates it";

#[path = "turn_cancel/stop.rs"]
mod stop;
#[path = "turn_cancel/window.rs"]
mod window;

/// Names the turn the Controller actually stopped, independently of any later queued turn.
pub(super) fn cancel(command: &str, turn: &str) -> ControllerToNodeMessage {
    ControllerToNodeMessage::CancelTurn(CancelTurnMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: start("").operation_id,
        execution_id: ExecutionId::new(EXECUTION),
        payload: CancelTurn {
            node_id: NodeId::new("test-node"),
            command_id: CommandId::new(command),
            turn_id: TurnId::new(turn),
        },
    })
}

/// Sends `hello`, optionally declaring `turn_cancel`, until the Node answers it or refuses it
/// for the takeover boundary. A previous connection's slot is released asynchronously, so a
/// connection turned away as busy is retried.
pub(super) async fn greet(
    fixture: &Fixture,
    turn_cancel: bool,
) -> (tokio::net::UnixStream, Option<NodeToControllerMessage>) {
    let log = fixture.path().join("agent-service.log");
    let refusals = || {
        fs::read_to_string(&log)
            .unwrap_or_default()
            .matches(REFUSAL)
            .count()
    };
    for _ in 0..100 {
        let before = refusals();
        let mut stream =
            tokio::net::UnixStream::connect(fixture.config().home_directory.join("control.sock"))
                .await
                .unwrap();
        let hello = ControllerToNodeMessage::Hello(HelloMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            payload: Hello {
                controller_id: ControllerId::new("owner"),
                supported_versions: vec![CURRENT_PROTOCOL_VERSION],
                thread_previews: false,
                turn_cancel,
            },
        });
        if write_controller_message(&mut stream, &hello).await.is_ok() {
            let answer = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                read_node_message(&mut stream),
            )
            .await
            .expect("Node answers or closes")
            .ok()
            .flatten();
            if answer.is_some() {
                return (stream, answer);
            }
            // The refusal is logged once the session closes.
            for _ in 0..20 {
                if refusals() > before {
                    return (stream, None);
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the Node neither answered nor refused hello")
}

/// Only a declared `turn_cancel` gets the capability; while a session that chose to keep is
/// unacknowledged, a Controller without it is refused before any event, and one with it then
/// receives every event from the smallest unacknowledged sequence.
#[test]
fn a_keep_session_is_taken_over_only_by_a_controller_that_negotiated_turn_cancel() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (stream, answer) = greet(&fixture, /*turn_cancel*/ false).await;
                let Some(NodeToControllerMessage::HelloAccepted(plain)) = answer else {
                    panic!("expected handshake, got {answer:?}")
                };
                assert_eq!(
                    (
                        plain.payload.turn_cancel,
                        plain
                            .payload
                            .capabilities
                            .contains(&NodeCapability::TurnCancel)
                    ),
                    (false, false)
                );
                drop(stream);
                let (mut stream, answer) = greet(&fixture, /*turn_cancel*/ true).await;
                let Some(NodeToControllerMessage::HelloAccepted(negotiated)) = answer else {
                    panic!("expected handshake, got {answer:?}")
                };
                assert_eq!(
                    (
                        negotiated.payload.turn_cancel,
                        negotiated
                            .payload
                            .capabilities
                            .contains(&NodeCapability::TurnCancel)
                    ),
                    (true, true)
                );
                let mut keeping = start("[hold]");
                keeping.payload.spec.keep_session_on_cancel = true;
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(keeping),
                )
                .await;
                let first = loop {
                    if let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await
                    {
                        break event.sequence;
                    }
                };
                drop(stream);

                let (_refused, answer) = greet(&fixture, /*turn_cancel*/ false).await;
                assert!(answer.is_none(), "an old Controller saw {answer:?}");

                let (mut stream, answer) = greet(&fixture, /*turn_cancel*/ true).await;
                assert!(matches!(
                    answer,
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
                let replayed = loop {
                    if let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await
                    {
                        break event.sequence;
                    }
                };
                assert_eq!(replayed, first);
                send(&mut stream, end()).await;
                loop {
                    if let NodeToControllerMessage::AgentSessionEnded(_) =
                        receive(&mut stream).await
                    {
                        break;
                    }
                }
            });
        child.terminate();
    });
}

/// A connection that did not negotiate `turn_cancel` cannot start a session that chose a keep
/// behaviour: the start is refused before durable admission, so nothing is left that would later
/// lock that Controller out.
#[test]
fn a_keep_session_cannot_be_started_without_turn_cancel() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, answer) = greet(&fixture, /*turn_cancel*/ false).await;
                assert!(matches!(
                    answer,
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
                let mut keeping = start("[hold]");
                keeping.payload.spec.keep_session_on_cancel = true;
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(keeping),
                )
                .await;
                let closed = loop {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        read_node_message(&mut stream),
                    )
                    .await
                    .expect("the Node answers or closes")
                    {
                        Ok(Some(NodeToControllerMessage::Heartbeat(_))) => {}
                        Ok(Some(other)) => break Some(other),
                        Ok(None) | Err(_) => break None,
                    }
                };
                assert!(closed.is_none(), "the start was answered with {closed:?}");
                drop(stream);
                // Nothing was admitted, so the same Controller is not refused afterwards.
                let (_stream, answer) = greet(&fixture, /*turn_cancel*/ false).await;
                assert!(matches!(
                    answer,
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
            });
        child.terminate();
    });
}

/// A cancel whose acceptance reply never reaches the Controller is still settled: the durable
/// acceptance, not the reply, wakes the idle session, and the settlement waits in the outbox.
#[test]
fn a_cancel_is_settled_even_when_its_reply_is_lost() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = greet(&fixture, /*turn_cancel*/ true).await;
                let mut keeping = start("hello");
                keeping.payload.spec.keep_session_on_cancel = true;
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(keeping),
                )
                .await;
                // The initial turn ends; the session now waits for commands.
                loop {
                    if let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await
                        && event
                            .payload
                            .record
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            == Some("turnEnded")
                    {
                        break;
                    }
                }
                drop(stream);
                // Each attempt sends a fresh cancel and disconnects at once, so the accepted reply
                // cannot be written. A disconnect can also win before the Node reads the frame, and
                // then nothing was accepted; only then is the next attempt needed. Before the fix no
                // attempt is ever settled, because only a delivered reply woke the idle session.
                let mut settled = None;
                let mut stream = None;
                for attempt in 0..5 {
                    let (mut sending, _) = greet(&fixture, /*turn_cancel*/ true).await;
                    let command = CommandId::new(format!("cancel-unknown-{attempt}"));
                    send(
                        &mut sending,
                        ControllerToNodeMessage::CancelTurn(CancelTurnMessage {
                            protocol_version: CURRENT_PROTOCOL_VERSION,
                            operation_id: start("").operation_id,
                            execution_id: ExecutionId::new(EXECUTION),
                            payload: CancelTurn {
                                node_id: NodeId::new("test-node"),
                                command_id: command.clone(),
                                turn_id: TurnId::new("never-submitted"),
                            },
                        }),
                    )
                    .await;
                    drop(sending);
                    let (mut reading, _) = greet(&fixture, /*turn_cancel*/ true).await;
                    let found = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        loop {
                            if let NodeToControllerMessage::TurnCancelSettled(message) =
                                receive(&mut reading).await
                            {
                                break message.payload;
                            }
                        }
                    })
                    .await;
                    if let Ok(payload) = found {
                        settled = Some(payload);
                        stream = Some(reading);
                        break;
                    }
                    drop(reading);
                }
                let settled = settled.expect("an accepted cancel whose reply was lost is settled");
                let mut stream = stream.expect("the reading connection");
                assert_eq!(
                    (settled.turn_id, settled.outcome),
                    (TurnId::new("never-submitted"), TurnCancelOutcome::Unknown)
                );
                send(&mut stream, end()).await;
                loop {
                    if let NodeToControllerMessage::AgentSessionEnded(_) =
                        receive(&mut stream).await
                    {
                        break;
                    }
                }
            });
        child.terminate();
    });
}
