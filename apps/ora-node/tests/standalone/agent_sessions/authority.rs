use super::super::runtime_fixture;
use super::*;
use pretty_assertions::assert_eq;

/// Controlled start uses a fresh exact permit; closure prevents new commands even over local IPC.
#[test]
fn controlled_start_and_command_closure_use_original_runtime_authority() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let mut child = launch(&fixture, &config);
        runtime_fixture::advance_epoch(fixture.path());
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let (mut stream, identity) = connect(&fixture).await;
                let mut binding = runtime_fixture::binding(fixture.path(), &identity);
                send(
                    &mut stream,
                    ControllerToNodeMessage::BindRuntime(binding.clone()),
                )
                .await;
                assert!(matches!(
                    receive(&mut stream).await,
                    NodeToControllerMessage::RuntimeControlState(_)
                ));
                let input = start("[hold]");
                let mut permit = binding.clone();
                permit.execution_id = input.execution_id.as_str().into();
                permit.node_operation_id = input.operation_id.as_str().into();
                let envelope = ControllerToNodeMessage::ControlledStartAgentSession(Box::new(
                    ControlledStartAgentSession {
                        binding: permit,
                        command: input,
                    },
                ));
                send(&mut stream, envelope.clone()).await;
                let mut has_event = false;
                let mut has_state = false;
                while !has_event || !has_state {
                    match receive(&mut stream).await {
                        NodeToControllerMessage::ExecutionStatus(status) => {
                            assert_eq!(status.payload.state, ExecutionState::Running);
                            has_state = true;
                        }
                        NodeToControllerMessage::ThreadEvent(_) => has_event = true,
                        other => panic!("unexpected {other:?}"),
                    }
                }
                send(&mut stream, envelope).await;
                loop {
                    if let NodeToControllerMessage::ExecutionStatus(status) =
                        receive(&mut stream).await
                    {
                        assert_eq!(status.payload.state, ExecutionState::Running);
                        break;
                    }
                }
                binding.input_closed = true;
                binding.control_version += 1;
                send(&mut stream, ControllerToNodeMessage::BindRuntime(binding)).await;
                loop {
                    if let NodeToControllerMessage::RuntimeControlState(state) =
                        receive(&mut stream).await
                    {
                        assert_eq!(state.unfinished_execution_ids, vec![EXECUTION]);
                        break;
                    }
                }
                send(&mut stream, turn()).await;
                tokio::time::timeout(std::time::Duration::from_secs(/*secs*/ 5), async {
                    loop {
                        match read_node_message(&mut stream).await {
                            Ok(None) | Err(_) => break,
                            Ok(Some(NodeToControllerMessage::SessionCommandAccepted(_))) => {
                                panic!("closed input accepted")
                            }
                            Ok(Some(_)) => {}
                        }
                    }
                })
                .await
                .unwrap();
            });
        child.terminate();
        let db = ora_node_db::NodeDatabase::open(
            &fixture.config().home_directory.join("ora-node.sqlite3"),
            fixture.config().identity,
        )
        .unwrap();
        assert_eq!(
            db.session_journal()
                .unwrap()
                .command_state(
                    &ExecutionId::new(EXECUTION),
                    &CommandId::new("second-command")
                )
                .unwrap(),
            None
        );
    });
}
