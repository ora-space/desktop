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
            let mut stream = super::opened(
                fixture
                    .manager
                    .prompt_session(prompt("default"))
                    .await
                    .unwrap(),
            )
            .await;
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

        let (mut actor, _commands) = unrun_actor(&fixture, session);
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

/// Builds the actor of `session` without running it, so a test drives its steps one by one.
fn unrun_actor(
    fixture: &Fixture,
    session: Session,
) -> (
    crate::RuntimeActor<TestHost>,
    mpsc::UnboundedSender<crate::command::RuntimeCommand>,
) {
    let inner = &fixture.manager.inner;
    let (commands, receiver) = mpsc::unbounded_channel();
    let actor = crate::RuntimeActor::<TestHost> {
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
        held_provider: false,
        commands_exceed_limit: false,
        scheduler: inner.scheduler.clone(),
        events: inner.events.clone(),
        title_acquisition: crate::TitleAcquisition::disabled(),
        command_sender: commands.downgrade(),
        session_mcp: NoSessionMcp::default(),
        skill_catalog: inner.skill_catalog.clone(),
        announced_skills: None,
        barriers: inner.barriers.clone(),
        live_mcp: LiveMcpState::Inactive,
        turn_cancel: Default::default(),
        #[cfg(feature = "test-support")]
        test_prompt_inactivity_windows: None,
        exit_probe: None,
    };
    (actor, commands)
}

/// What a change's caller is eventually told.
type Outcome = oneshot::Receiver<crate::config_changes::ConfigChangeOutcome>;

/// A turn started on `answered` without an actor, and the parts a test needs to settle it.
struct UnrunTurn {
    fixture: Fixture,
    actor: crate::RuntimeActor<TestHost>,
    _commands: mpsc::UnboundedSender<crate::command::RuntimeCommand>,
    /// Until a step that consumes them takes them.
    channel: Option<SessionChannel>,
    client: crate::connection::AgentAcpClient,
    pending: Option<ora_acp::PendingSessionRequest<PromptResponse>>,
    /// The prompt as the agent received it.
    prompt: Value,
}

impl UnrunTurn {
    /// Opens the session's channel and writes a prompt to it, which the agent holds open.
    async fn start() -> Self {
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
        let prompt = fixture.next_frame().await;
        assert_eq!(prompt["method"], "session/prompt");
        let (actor, commands) = unrun_actor(&fixture, session);
        Self {
            fixture,
            actor,
            _commands: commands,
            channel: Some(channel),
            client,
            pending: Some(pending),
            prompt,
        }
    }

    /// Asks the agent for model `a` and returns the request it was sent, unanswered.
    async fn change(&mut self) -> (Outcome, Value) {
        let (accepted, route) = oneshot::channel();
        self.actor
            .change_config(
                self.channel.as_mut(),
                crate::command::ConfigChangeRequest {
                    config_id: agent_client_protocol_schema::v1::SessionConfigId::new("model"),
                    value: agent_client_protocol_schema::v1::SessionConfigOptionValue::value_id(
                        "a",
                    ),
                    accepted,
                },
            )
            .await;
        let crate::config_changes::ConfigChangeRoute::Asked { write, outcome } =
            route.await.unwrap().unwrap();
        assert!(write.is_none(), "the agent reads, so the actor wrote it");
        let request = self.fixture.next_frame().await;
        assert_eq!(request["method"], "session/set_config_option");
        (outcome, request)
    }

    /// Has the agent answer a change with model `a`.
    fn answer(&self, request: &Value) {
        let options = json!([{"id":"model", "name":"Model", "category":"model", "type":"select",
            "currentValue":"a", "options":[{"value":"a","name":"A"}]}]);
        let answer =
            json!({"jsonrpc":"2.0", "id":request["id"], "result":{"configOptions":options}});
        self.fixture
            .agent
            .send(AgentInput::RawFrame(answer))
            .unwrap();
    }

    /// Has the agent end the turn as cancelled, and lets its frames queue up unread.
    async fn cancelled(&self) {
        self.fixture
            .agent
            .send(AgentInput::Finish(
                "answered".to_string(),
                StopReason::Cancelled,
            ))
            .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    /// Has the agent refuse the prompt with an error it could recover from, and lets it queue up.
    async fn refused(&self) {
        let refusal = json!({"jsonrpc":"2.0", "id":self.prompt["id"],
            "error":{"code":-32000, "message":"refused"}});
        self.fixture
            .agent
            .send(AgentInput::RawFrame(refusal))
            .unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    /// Settles the turn as a cancel past its grace, telling `events` how it ended.
    async fn settle(&mut self, events: &mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>) {
        self.actor
            .turn_cancel
            .request(Instant::now() + Duration::from_secs(15));
        self.stop(events).await;
    }

    /// Settles the turn as one stopped without a cancel deadline, telling `events` how it ended.
    async fn stop(&mut self, events: &mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>) {
        let channel = self.channel.take().unwrap();
        let pending = self.pending.take().unwrap();
        let end_report = crate::PromptEndReport::new();
        self.actor
            .settle_prompt_cancel(
                channel,
                pending,
                crate::turn_cancel::PromptStop {
                    operation_id: 1,
                    client: &self.client,
                    permissions: &HashMap::new(),
                    events,
                    end_report: &end_report,
                    followers: &mut crate::session_followers::SessionFollowers::new(),
                    tool_timings: &crate::tool_timing::ToolTimings::default(),
                    saw_output: &mut false,
                },
                /*notify_owner*/ true,
                /*completion*/ None,
            )
            .await;
    }
}

/// Waits for a change's caller to be told, and names the model the actor is on then.
async fn told(outcome: Outcome, actor: &crate::RuntimeActor<TestHost>) -> Option<String> {
    outcome.await.unwrap().unwrap();
    crate::model_config::current_model(&actor.reported_config_options)
}

/// An answer already queued when a cancel is about to be written is read ahead of writing it, and
/// taken in its turn rather than dropped with the turn's other traffic.
#[test]
fn a_change_answered_before_the_cancel_is_written_is_taken() {
    run_case(async {
        let mut turn = UnrunTurn::start().await;
        let (outcome, request) = turn.change().await;
        turn.answer(&request);
        turn.cancelled().await;
        let (events, _owner) = mpsc::channel(/*buffer*/ 8);

        turn.settle(&events).await;

        assert_eq!(told(outcome, &turn.actor).await, Some("a".to_string()));
        assert_eq!(
            crate::model_config::current_model(&turn.actor.reported_config_options),
            Some("a".to_string())
        );
        drop(turn.actor);
        turn.fixture.shutdown().await;
    });
}

/// So is one still queued when the cancel's grace has run out and the queue is drained.
#[test]
fn a_change_answered_before_the_queue_is_drained_is_taken() {
    run_case(async {
        let mut turn = UnrunTurn::start().await;
        let (outcome, request) = turn.change().await;
        turn.answer(&request);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let (events, _owner) = mpsc::channel(/*buffer*/ 8);

        crate::events::drain_queued_prompt_events(
            &mut turn.actor,
            turn.channel.as_mut().unwrap(),
            &turn.client,
            &events,
            &mut false,
            &mut crate::session_followers::SessionFollowers::new(),
        )
        .await;

        assert_eq!(told(outcome, &turn.actor).await, Some("a".to_string()));
        drop(turn.actor);
        turn.fixture.shutdown().await;
    });
}

/// A cancelled turn whose receiver left takes the provider with it, but a change still in flight
/// is made rather than failed: nobody gave up on the agent.
#[test]
fn a_change_in_flight_is_made_when_a_cancelled_turn_has_nobody_to_tell() {
    run_case(async {
        let mut turn = UnrunTurn::start().await;
        let (outcome, _request) = turn.change().await;
        turn.cancelled().await;
        let (events, owner) = mpsc::channel(/*buffer*/ 8);
        drop(owner);

        turn.settle(&events).await;

        assert!(outcome.await.unwrap().is_ok());
        assert!(turn.actor.channel.is_none());
        drop(turn.actor);
        turn.fixture.shutdown().await;
    });
}

/// How a stopped turn ends once its owner has gone.
#[derive(Clone, Copy, Debug)]
enum StoppedTurn {
    /// The agent confirms the cancel.
    Cancelled,
    /// The agent refuses the prompt with an error it could recover from.
    Refused,
}

/// A turn stopped while nobody reads it any more takes the provider with it, but a change still in
/// flight is made rather than failed: the agent was not lost.
#[test]
fn a_change_in_flight_is_made_when_a_stopped_turn_has_nobody_to_tell() {
    for end in [StoppedTurn::Cancelled, StoppedTurn::Refused] {
        run_case(async move {
            let mut turn = UnrunTurn::start().await;
            let (outcome, _request) = turn.change().await;
            match end {
                StoppedTurn::Cancelled => turn.cancelled().await,
                StoppedTurn::Refused => turn.refused().await,
            }
            let (events, owner) = mpsc::channel(/*buffer*/ 8);
            drop(owner);

            turn.stop(&events).await;

            assert!(outcome.await.unwrap().is_ok(), "{end:?}");
            assert!(turn.actor.channel.is_none());
            drop(turn.actor);
            turn.fixture.shutdown().await;
        });
    }
}

/// A turn cancel that finds the agent already refused the prompt ends the turn as that refusal;
/// with its owner gone, the change still in flight is made, not failed.
#[test]
fn a_change_in_flight_is_made_when_a_cancel_finds_a_refusal_nobody_reads() {
    run_case(async {
        let mut turn = UnrunTurn::start().await;
        let (outcome, _request) = turn.change().await;
        turn.refused().await;
        let (events, owner) = mpsc::channel(/*buffer*/ 8);
        drop(owner);

        // With a turn-cancel deadline, so the queued refusal is read before any cancel is written.
        turn.settle(&events).await;

        assert!(outcome.await.unwrap().is_ok());
        assert!(turn.actor.channel.is_none());
        assert!(
            turn.fixture.frames.try_recv().is_err(),
            "a cancel was written"
        );
        drop(turn.actor);
        turn.fixture.shutdown().await;
    });
}
