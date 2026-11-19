//! The `turn_cancel` handshake and the takeover boundary of sessions that chose a keep behaviour.
//!
//! Evidence for specs/test-cases/node/protocol/cancel-turn-and-keep-session.md cases 16 and 17.
use super::*;
use pretty_assertions::assert_eq;

/// The reason the Node logs when it refuses a takeover; it closes the stream without a frame.
const REFUSAL: &str = "sessions that chose turn_cancel need a Controller that negotiates it";

/// Sends `hello`, optionally declaring `turn_cancel`, until the Node answers it or refuses it
/// for the takeover boundary. A previous connection's slot is released asynchronously, so a
/// connection turned away as busy is retried.
async fn greet(
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
