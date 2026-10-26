use super::*;
use pretty_assertions::assert_eq;

/// A held Agent response reaches the real socket before it becomes a durable message record.
#[test]
fn negotiated_preview_streams_before_the_agent_turn_finishes() {
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
                let mut stream = tokio::net::UnixStream::connect(
                    fixture.config().home_directory.join("control.sock"),
                )
                .await
                .unwrap();
                send(
                    &mut stream,
                    ControllerToNodeMessage::Hello(HelloMessage {
                        protocol_version: CURRENT_PROTOCOL_VERSION,
                        payload: Hello {
                            execution_non_acceptance: false,
                            controller_id: ControllerId::new("owner"),
                            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
                            thread_previews: true,
                        },
                    }),
                )
                .await;
                let NodeToControllerMessage::HelloAccepted(hello) = receive(&mut stream).await
                else {
                    panic!("expected handshake")
                };
                assert!(hello.payload.thread_previews);
                send(
                    &mut stream,
                    ControllerToNodeMessage::StartAgentSession(start("[hold]")),
                )
                .await;
                loop {
                    match receive(&mut stream).await {
                        NodeToControllerMessage::ThreadPreview(preview)
                            if serde_json::to_string(&preview.payload)
                                .unwrap()
                                .contains("holding") =>
                        {
                            assert_eq!(preview.execution_id, ExecutionId::new(EXECUTION));
                            assert!(
                                !history(&fixture)
                                    .iter()
                                    .any(|record| record.to_string().contains("holding"))
                            );
                            break;
                        }
                        NodeToControllerMessage::ThreadEvent(event) => {
                            assert_ne!(
                                event
                                    .payload
                                    .record
                                    .get("type")
                                    .and_then(serde_json::Value::as_str),
                                Some("turnEnded")
                            );
                        }
                        NodeToControllerMessage::ThreadPreview(_)
                        | NodeToControllerMessage::ExecutionStatus(_) => {}
                        other => panic!("unexpected {other:?}"),
                    }
                }
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
        assert!(
            history(&fixture)
                .iter()
                .any(|record| record.to_string().contains("holding"))
        );
    });
}
