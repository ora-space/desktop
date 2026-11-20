//! Case 14: the protocol window delays delivery, never the durable cancel conclusion.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn a_full_window_does_not_block_cancel_settlement() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = GitServerFixture::new(fixture.path(), fixture.path().join("main/.git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let (mut stream, answer) = greet(&fixture, /*turn_cancel*/ true).await;
            assert!(matches!(answer, Some(NodeToControllerMessage::HelloAccepted(_))));
            let mut input = start("[hold-many]");
            input.payload.spec.keep_session_on_cancel = true;
            send(&mut stream, ControllerToNodeMessage::StartAgentSession(input)).await;
            loop {
                if matches!(receive(&mut stream).await, NodeToControllerMessage::ThreadEvent(_)) { break; }
            }
            let path = fixture.config().home_directory.join("ora-node.sqlite3");
            let db = rusqlite::Connection::open(&path).unwrap();
            let mut sent = 1;
            while sent < 256 {
                let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await else { panic!("expected ordered event") };
                sent += 1;
                assert_eq!(event.sequence.value(), sent);
            }
            let accepted_at = std::time::Instant::now();
            send(&mut stream, cancel("cancel-full", "first")).await;
            loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::SessionCommandAccepted(_) => break,
                    other => panic!("full window emitted {other:?}"),
                }
            }
            let settlement = tokio::time::timeout(std::time::Duration::from_secs(17), async {
                loop {
                    let mut statement = db.prepare("SELECT event FROM execution_events WHERE execution=?1 ORDER BY sequence").unwrap();
                    let events = statement.query_map([EXECUTION], |row| row.get::<_,String>(0)).unwrap();
                    if let Some(event) = events.map(|row| serde_json::from_str::<NodeToControllerMessage>(&row.unwrap()).unwrap()).find(|event| matches!(event, NodeToControllerMessage::TurnCancelSettled(_))) { break event; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.expect("local G+Q does not depend on ACKs");
            assert!(accepted_at.elapsed() < std::time::Duration::from_secs(17));
            let NodeToControllerMessage::TurnCancelSettled(expected) = settlement else { unreachable!() };
            assert_eq!(expected.payload.outcome, TurnCancelOutcome::SessionEnding);
            // Queries still work while no event can be delivered.
            send(&mut stream, query()).await;
            assert!(matches!(receive(&mut stream).await, NodeToControllerMessage::ExecutionStatus(_)));
            send(&mut stream, ack(1)).await;
            loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::ThreadEvent(event) => {
                        sent += 1;
                        assert_eq!(event.sequence.value(), sent);
                        send(&mut stream, ack(sent)).await;
                    }
                    NodeToControllerMessage::TurnCancelSettled(event) => {
                        sent += 1;
                        assert_eq!(event, expected);
                        assert_eq!(event.sequence.value(), sent);
                        break;
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        child.terminate();
    });
}
