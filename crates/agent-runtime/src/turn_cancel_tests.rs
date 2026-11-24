//! Turn cancels with a caller deadline, through the production actor and a fake provider.
//!
//! Evidence for specs/test-cases/node/protocol/cancel-turn-and-keep-session.md cases 24 and 25.
use super::*;
use agent_client_protocol_schema::v1::SessionId as AcpSessionId;
use agent_client_protocol_schema::v1::{
    AGENT_METHOD_NAMES, AvailableCommand, AvailableCommandsUpdate, MessageId, PromptRequest,
};
use pretty_assertions::assert_eq;
use tokio::time::Instant;

/// Case 24: a cancel that arrives after admission committed but before the prompt is written
/// keeps the prompt from the Agent, ends the turn cancelled, and leaves the session reusable.
#[test]
fn a_cancel_before_the_prompt_write_never_reaches_the_agent() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("unsent");
        let manager = fixture.manager.clone();
        let mut stream = fixture
            .manager
            .prompt_session_with_admission(prompt("unsent"), MessageId::new("turn-1"), move || {
                // The commit point is the last moment before the write; the cancel lands there.
                manager.cancel_turn_prompt("unsent", Instant::now() + Duration::from_secs(15))
            })
            .await
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::Cancelled);
        assert!(stream.end_report().is_some_and(|report| report.unsent()));
        assert!(
            stream
                .end_report()
                .is_some_and(|report| report.cancel_applied())
        );
        assert!(fixture.frames.try_recv().is_err(), "no prompt was written");
        drop(stream);
        let mut next = fixture
            .manager
            .prompt_session(prompt("unsent"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        fixture
            .agent
            .send(AgentInput::Finish("unsent".into(), StopReason::EndTurn))
            .unwrap();
        assert_eq!(completion(&mut next).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// Case 25: a turn cancel merged into a watchdog stop replaces the watchdog's short grace with its
/// own deadline, once; the prompt is never resent, and the deadline decides the outcome.
#[test]
fn a_cancel_merged_into_a_watchdog_stop_owns_its_deadline() {
    run_case(async {
        for answered in [true, false] {
            let mut fixture = Fixture::new().await;
            fixture.add_session("default");
            let (configured, confirmed) = oneshot::channel();
            fixture
                .agent
                .send(AgentInput::CancelReply {
                    session_id: "default".to_string(),
                    reply: CancelReply::Ignore,
                    configured,
                })
                .unwrap();
            confirmed.await.unwrap();
            let mut stream = fixture
                .manager
                .prompt_session(prompt("default"))
                .await
                .unwrap();
            assert_eq!(fixture.next_frame().await["method"], "session/prompt");
            tokio::time::advance(Duration::from_secs(45)).await;
            assert_eq!(fixture.next_frame().await["method"], "session/cancel");
            let deadline = Instant::now() + Duration::from_secs(15);
            fixture
                .manager
                .cancel_turn_prompt("default", deadline)
                .unwrap();
            // A later cancel never moves the deadline the first one fixed.
            fixture
                .manager
                .cancel_turn_prompt("default", deadline + Duration::from_secs(60))
                .unwrap();
            // Past the watchdog's own grace, the merged cancel still waits.
            tokio::time::advance(Duration::from_secs(10)).await;
            tokio::task::yield_now().await;
            assert!(stream.try_recv().is_none());
            if answered {
                fixture
                    .agent
                    .send(AgentInput::Finish("default".into(), StopReason::Cancelled))
                    .unwrap();
                assert_eq!(completion(&mut stream).await, StopReason::Cancelled);
                assert_eq!(stream.end_cause(), Some(PromptEndCause::UserStopped));
            } else {
                tokio::time::advance(Duration::from_secs(6)).await;
                let error = loop {
                    match stream.recv().await.expect("the prompt ends") {
                        Err(error) => break error,
                        Ok(_) => {}
                    }
                };
                assert!(error.is_turn_cancel_unanswered());
            }
            assert!(
                fixture.frames.try_recv().is_err(),
                "the prompt is never resent"
            );
            fixture.shutdown().await;
        }
    });
}

/// An answer that arrives once `session/cancel` is written answers the cancel, even if the Agent
/// calls it `end_turn`: the turn is reported `cancelled` so its owner judges the stop.
#[test]
fn an_end_turn_answer_to_a_written_cancel_is_reported_cancelled() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("default");
        let (configured, confirmed) = oneshot::channel();
        fixture
            .agent
            .send(AgentInput::CancelReply {
                session_id: "default".to_string(),
                reply: CancelReply::EndTurn,
                configured,
            })
            .unwrap();
        confirmed.await.unwrap();
        let mut stream = fixture
            .manager
            .prompt_session(prompt("default"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        fixture
            .manager
            .cancel_turn_prompt("default", Instant::now() + Duration::from_secs(15))
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(completion(&mut stream).await, StopReason::Cancelled);
        assert!(
            stream
                .end_report()
                .is_some_and(|report| report.cancel_applied())
        );
        fixture.shutdown().await;
    });
}

/// A prompt that ends on its own reports that no cancel acted on it, even when its own stop
/// reason is `cancelled` and a cancel was requested right after: its owner must not judge it as
/// a stop the cancel caused.
#[test]
fn a_prompt_that_ended_on_its_own_reports_no_applied_cancel() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("default");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("default"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        fixture
            .agent
            .send(AgentInput::Finish("default".into(), StopReason::Cancelled))
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::Cancelled);
        fixture
            .manager
            .cancel_turn_prompt("default", Instant::now() + Duration::from_secs(15))
            .unwrap();
        tokio::task::yield_now().await;
        assert!(
            !stream
                .end_report()
                .is_some_and(|report| report.cancel_applied())
        );
        assert!(fixture.frames.try_recv().is_err(), "no cancel is written");
        fixture.shutdown().await;
    });
}

/// An answer that was already readable keeps its session, so a catalog queued ahead of that answer
/// is still the one the session last reported.
#[test]
fn a_catalog_ahead_of_an_already_queued_answer_is_kept() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        let session = fixture.seed_session("answered");
        let channel = fixture
            .supervisor
            .open_session_channel("answered", "answered")
            .unwrap();
        let client = channel.connection.client.clone();
        let pending = client
            .start_session_request::<_, PromptResponse>(
                AcpSessionId::new("answered"),
                AGENT_METHOD_NAMES.session_prompt,
                &PromptRequest::new("answered", Vec::new()),
            )
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        let update = AvailableCommandsUpdate::new(vec![AvailableCommand::new("late", "Runs it")]);
        fixture
            .agent
            .send(AgentInput::Update(SessionNotification::new(
                "answered",
                SessionUpdate::AvailableCommandsUpdate(update.clone()),
            )))
            .unwrap();
        fixture
            .agent
            .send(AgentInput::Finish(
                "answered".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        // No actor reads the channel, so once everything is quiet both are queued on it.
        tokio::time::sleep(Duration::from_secs(1)).await;

        let inner = &fixture.manager.inner;
        let (commands, receiver) = mpsc::unbounded_channel();
        let mut actor = crate::RuntimeActor::<TestHost> {
            cleanup_outcomes: HashMap::new(),
            recorder: fixture.manager.open_recorder(&session).unwrap().recorder,
            session,
            cwd: fixture._directory.path().to_path_buf(),
            repository: inner.store.clone(),
            clock: inner.clock,
            connection: fixture.supervisor.clone(),
            channel: None,
            commands: receiver,
            sessions_root: inner.sessions_root.clone(),
            connections: inner.connections.clone(),
            handoff: crate::HandoffDebt::Settled,
            rebuilt_binding: None,
            load_restoration: Default::default(),
            reported_config_options: Vec::new(),
            scheduler: inner.scheduler.clone(),
            events: inner.events.clone(),
            title_acquisition: crate::TitleAcquisition::disabled(),
            command_sender: commands.downgrade(),
            session_mcp: NoSessionMcp::default(),
            barriers: inner.barriers.clone(),
            live_mcp: LiveMcpState::Inactive,
            turn_cancel: Default::default(),
            #[cfg(feature = "test-support")]
            test_prompt_inactivity_windows: None,
            exit_probe: None,
        };
        actor
            .turn_cancel
            .request(Instant::now() + Duration::from_secs(15));
        let (events, _owner) = mpsc::channel(/*buffer*/ 8);
        let end_report = crate::PromptEndReport::new();
        actor
            .settle_prompt_cancel(
                channel,
                pending,
                crate::turn_cancel::PromptStop {
                    operation_id: 1,
                    client: &client,
                    permissions: &HashMap::new(),
                    events: &events,
                    end_report: &end_report,
                    followers: &mut crate::session_followers::SessionFollowers::new(),
                    tool_timings: &crate::tool_timing::ToolTimings::default(),
                    saw_output: &mut false,
                },
                /*notify_owner*/ true,
                /*completion*/ None,
            )
            .await;

        assert!(!end_report.cancel_applied(), "the answer won the race");
        assert_eq!(
            actor
                .channel
                .as_ref()
                .and_then(|channel| channel.reported_commands()),
            Some(SessionUpdate::AvailableCommandsUpdate(update))
        );
        drop(actor);
        fixture.shutdown().await;
    });
}
