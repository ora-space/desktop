use super::*;
use crate::support::until;
use pretty_assertions::assert_eq;

/// SIGKILL leaves replayable events, and recovery adds exactly one interrupted terminal before commands.
#[test]
fn killed_session_replays_and_ends_before_accepting_commands() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let first = runtime.block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            send(
                &mut stream,
                ControllerToNodeMessage::StartAgentSession(start("[hold]")),
            )
            .await;
            loop {
                if let NodeToControllerMessage::ThreadEvent(event) = receive(&mut stream).await {
                    break event;
                }
            }
        });
        let process = plugin(&root);
        child.kill();
        let mut replacement = launch(&fixture, &config);
        runtime.block_on(async {
            let (mut stream, _) = connect(&fixture).await;
            send(&mut stream, turn()).await;
            let mut events = Vec::new();
            let mut rejected = false;
            let mut ended = false;
            while !rejected || !ended {
                match receive(&mut stream).await {
                    NodeToControllerMessage::SessionCommandRejected(_) => rejected = true,
                    NodeToControllerMessage::ThreadEvent(event) => events.push(event),
                    NodeToControllerMessage::AgentSessionEnded(event) => {
                        let AgentSessionResult::AgentSessionEnded(result) = event.payload;
                        assert_eq!(result.reason, AgentSessionEndReason::Interrupted);
                        assert_eq!(event.sequence.value(), events.len() as u64 + 1);
                        ended = true;
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert_eq!(events.first(), Some(&first));
            assert_eq!(
                events
                    .iter()
                    .map(|event| event.sequence.value())
                    .collect::<Vec<_>>(),
                (1..=events.len() as u64).collect::<Vec<_>>()
            );
        });
        replacement.terminate();
        until(|| process.has_exited().unwrap());
        assert_eq!(
            fs::read_to_string(root.join("echo-agent.pids"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert!(!history(&fixture).is_empty());
    });
}

/// Graceful service stop cancels an active turn and waits for the plugin before releasing the DB lease.
#[test]
fn service_shutdown_stops_the_active_agent_and_persists_its_end() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        let root = prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, _) = connect(&fixture).await;
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(start("[hold]")),
                )
                .await;
                loop {
                    if matches!(
                        receive(&mut stream).await,
                        NodeToControllerMessage::ThreadEvent(_)
                    ) {
                        break;
                    }
                }
            });
        let process = plugin(&root);
        child.terminate();
        assert!(process.has_exited().unwrap());
        let db = ora_node_db::NodeDatabase::open(
            &fixture.config().home_directory.join("ora-node.sqlite3"),
            fixture.config().identity,
        )
        .unwrap();
        let ExecutionState::Completed(ExecutionResult::AgentSession(
            AgentSessionResult::AgentSessionEnded(ended),
        )) = db
            .execution_state(&start("").operation_id, &start("").execution_id)
            .unwrap()
        else {
            panic!("not ended")
        };
        assert_eq!(ended.reason, AgentSessionEndReason::Cancelled);
        assert!(db.recoverable_sessions().unwrap().is_empty());
    });
}
