use super::*;
use pretty_assertions::assert_eq;

/// A real connection stops at 256, frees only exact ACK slots, and restarts from the smallest unacked event.
#[test]
fn window_bounds_delivery_without_blocking_queries_and_reconnect_replays_exactly() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        fs::create_dir_all(&fixture.config().home_directory).unwrap();
        let path = fixture.config().home_directory.join("ora-node.sqlite3");
        {
            let mut db = ora_node_db::NodeDatabase::open(&path, fixture.config().identity).unwrap();
            db.bind_controller(&ControllerId::new("owner")).unwrap();
            db.accept_session(&start("seeded")).unwrap();
            db.start_session(&start("seeded"), &NodeIncarnationId::new("previous"))
                .unwrap();
            let journal = db.session_journal().unwrap();
            for number in 1..=300 {
                journal
                    .append_thread_event(
                        &ExecutionId::new(EXECUTION),
                        ThreadEvent {
                            turn_id: None,
                            record: serde_json::from_value(serde_json::json!({"number": number}))
                                .unwrap(),
                            truncated: false,
                        },
                    )
                    .unwrap();
            }
        }
        let mut child = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            for number in 1..=256 {
                let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await else {
                    panic!("expected thread")
                };
                assert_eq!(
                    (
                        event.sequence.value(),
                        event.payload.record["number"].as_u64()
                    ),
                    (number, Some(number))
                );
            }
            // Several replay ticks pass while the window is full; control replies remain live.
            for _ in 0..10 {
                let message = tokio::time::timeout(
                    std::time::Duration::from_secs(/*secs*/ 2),
                    read_node_message(&mut stream),
                )
                .await
                .unwrap()
                .unwrap()
                .unwrap();
                assert!(matches!(message, NodeToControllerMessage::Heartbeat(_)));
            }
            send(&mut stream, query()).await;
            assert!(matches!(
                receive(&mut stream).await,
                NodeToControllerMessage::ExecutionStatus(_)
            ));
            send(&mut stream, ack(2)).await;
            let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await else {
                panic!("slot not freed")
            };
            assert_eq!(event.sequence.value(), 257);
            send(&mut stream, ack(2)).await;
            send(&mut stream, query()).await;
            assert!(matches!(
                receive(&mut stream).await,
                NodeToControllerMessage::ExecutionStatus(_)
            ));
            for _ in 0..5 {
                assert!(matches!(
                    read_node_message(&mut stream).await.unwrap().unwrap(),
                    NodeToControllerMessage::Heartbeat(_)
                ));
            }
        });
        child.kill();
        let mut replacement = launch(&fixture, &config);
        runtime.block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            let mut expected = (1..=300).filter(|number| *number != 2);
            let mut buffered = std::collections::VecDeque::new();
            loop {
                let message = match buffered.pop_front() {
                    Some(message) => message,
                    None => receive(&mut stream).await,
                };
                match message {
                    NodeToControllerMessage::ThreadEvent(event) => {
                        let number = expected.next().expect("extra replay event");
                        assert_eq!(
                            (
                                event.sequence.value(),
                                event.payload.record["number"].as_u64()
                            ),
                            (number, Some(number))
                        );
                        send(&mut stream, ack(number)).await;
                        // A FIFO query barrier bounds ACK admission while retaining ordered events
                        // which the independent writer already queued before the status reply.
                        send(&mut stream, query()).await;
                        loop {
                            let message = receive(&mut stream).await;
                            if matches!(message, NodeToControllerMessage::ExecutionStatus(_)) {
                                break;
                            }
                            buffered.push_back(message);
                        }
                    }
                    NodeToControllerMessage::AgentSessionEnded(event) => {
                        assert_eq!(expected.next(), None);
                        assert_eq!(event.sequence.value(), 301);
                        send(&mut stream, ack(301)).await;
                        send(&mut stream, query()).await;
                        assert!(matches!(
                            receive(&mut stream).await,
                            NodeToControllerMessage::ExecutionStatus(_)
                        ));
                        break;
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        replacement.terminate();
        let db = ora_node_db::NodeDatabase::open(&path, fixture.config().identity).unwrap();
        assert!(db.pending_events().unwrap().is_empty());
        assert_eq!(
            db.find_session(&start("").operation_id, &start("").execution_id)
                .unwrap()
                .unwrap()
                .last_sequence,
            301
        );
    });
}
