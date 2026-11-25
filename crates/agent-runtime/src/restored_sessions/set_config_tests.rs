//! Covers what a view is told after the user configured a session whose agent says what changed only
//! in its answer.

use super::follower_setup_tests::{State, model, options, state};
use super::*;
use ora_contracts::{
    LoadSessionEvent, LoadSessionRequest, SetSessionConfigRequest, SetSessionConfigResponse,
    StopSessionRequest,
};
use pretty_assertions::assert_eq;

const SESSION: &str = "restored";

/// Reports every request to the test and sends the answers it is given.
async fn answered_by_the_test(
    mut answers: mpsc::UnboundedReceiver<Value>,
    mut reader: DuplexStream,
    mut writer: DuplexStream,
    frames: mpsc::UnboundedSender<Value>,
    exited: watch::Sender<bool>,
) {
    let registration = json!({"jsonrpc":"2.0","method":"ora/register",
        "params":{"methods":[],"emits":["agent/acp"]}});
    write_message(&mut writer, &registration).await.unwrap();
    loop {
        tokio::select! {
            envelope = read_message(&mut reader) => match envelope {
                Ok(Some(envelope)) if envelope["method"] != "ora/shutdown" => {
                    frames.send(envelope["params"].clone()).unwrap();
                }
                _ => break,
            },
            Some(answer) = answers.recv() => {
                let envelope = json!({"jsonrpc":"2.0", "method":"agent/acp", "params":answer});
                write_message(&mut writer, &envelope).await.unwrap();
            }
        }
    }
    exited.send_replace(true);
}

/// Drives one session whose agent answers only when told to.
struct Driven {
    fixture: ReviewDrainFixture,
    answers: mpsc::UnboundedSender<Value>,
}

impl Driven {
    /// Connects the agent without seeding a session.
    async fn new() -> Self {
        let (answers, given) = mpsc::unbounded_channel();
        let fixture = ReviewDrainFixture::new_with(move |reader, writer, _, frames, exited| {
            answered_by_the_test(given, reader, writer, frames, exited)
        })
        .await;
        Self { fixture, answers }
    }

    /// Waits for the agent to be asked this, and returns the request.
    async fn asked(&mut self, method: &str) -> Value {
        loop {
            let frame = self.fixture.next_frame().await;
            if frame["method"] == method {
                return frame;
            }
        }
    }

    /// Has the agent answer a request.
    fn answer(&self, request: &Value, result: Value) {
        let answer = json!({"jsonrpc":"2.0", "id":request["id"], "result":result});
        self.answers.send(answer).unwrap();
    }

    /// Has the agent refuse a request.
    fn fail(&self, request: &Value) {
        let failure = json!({"jsonrpc":"2.0", "id":request["id"],
            "error":{"code":-32000, "message":"stale session"}});
        self.answers.send(failure).unwrap();
    }

    /// Has the agent end a turn.
    fn end_turn(&self, request: &Value) {
        self.answer(request, json!(PromptResponse::new(StopReason::EndTurn)));
    }

    /// Sends a prompt without waiting for its admission.
    fn prompt(&self) -> tokio::task::JoinHandle<SessionEventStream<PromptSessionEvent>> {
        let manager = self.fixture.manager.clone();
        tokio::spawn(async move { manager.prompt_session(prompt(SESSION)).await.unwrap() })
    }

    /// Restores the session with these options and returns its first turn and that turn's request.
    async fn restored(
        &mut self,
        options: Value,
    ) -> (SessionEventStream<PromptSessionEvent>, Value) {
        let admitted = self.prompt();
        let restore = self.asked("session/load").await;
        self.answer(&restore, json!({"configOptions":options}));
        let turn = admitted.await.unwrap();
        let request = self.asked("session/prompt").await;
        (turn, request)
    }

    /// Chooses model `a`, and returns the choice in flight and the request that carries it.
    async fn choose(&mut self) -> (Choice, Value) {
        let manager = self.fixture.manager.clone();
        let choice = tokio::spawn(async move { manager.set_session_config(change()).await });
        let request = self.asked("session/set_config_option").await;
        (choice, request)
    }

    /// Has the agent push its options on its own.
    fn push(&self, current: &str) {
        self.answers.send(options(SESSION, current)).unwrap();
    }

    /// Has the agent answer the choice with model `a`, as `accept` does, without waiting.
    fn answer_choice(&self, request: &Value) {
        self.answer(request, json!({"configOptions":model("a")}));
    }

    /// Has the agent answer the choice with model `a`, and returns what its caller is told.
    async fn accept(&self, choice: Choice, request: &Value) -> Option<String> {
        self.answer_choice(request);
        chosen(choice).await
    }

    /// Opens the session.
    async fn load(&self) -> SessionEventStream<LoadSessionEvent> {
        self.fixture
            .manager
            .load_session(LoadSessionRequest {
                session_id: SESSION.into(),
            })
            .await
            .unwrap()
    }
}

/// A choice in flight.
type Choice = tokio::task::JoinHandle<Result<SetSessionConfigResponse, crate::RuntimeError>>;

/// Waits for a choice and names the model its caller is told.
async fn chosen(choice: Choice) -> Option<String> {
    crate::model_config::current_model(&choice.await.unwrap().unwrap().config_options)
}

/// Reads the session state a load delivers, up to its end.
async fn loaded(mut stream: SessionEventStream<LoadSessionEvent>) -> Vec<State> {
    let mut states = Vec::new();
    while let Some(event) = stream.recv().await {
        match event.unwrap() {
            LoadSessionEvent::SessionUpdate { update, .. } => states.extend(state(update)),
            LoadSessionEvent::Completed { .. } => break,
            _ => {}
        }
    }
    states
}

/// Names the model a view is on.
fn on(model: &str) -> Vec<State> {
    vec![State::Model(model.to_string())]
}

/// A view that opens the session learns of the change, whether or not a turn is running.
#[test]
fn a_view_opened_after_a_change_is_told_about_it() {
    for mid_turn in [true, false] {
        run_case(async move {
            let mut driven = Driven::new().await;
            driven.fixture.add_session(SESSION);
            let (mut turn, running) = driven.restored(model("b")).await;
            if !mid_turn {
                driven.end_turn(&running);
                assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
            }

            let (choice, request) = driven.choose().await;
            assert_eq!(driven.accept(choice, &request).await, Some("a".to_string()));
            let view = driven.load().await;
            if mid_turn {
                driven.end_turn(&running);
                assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
            }

            assert_eq!(loaded(view).await, on("a"));
            drop(turn);
            driven.fixture.shutdown().await;
        });
    }
}

/// A title poll that borrows the idle session when the answer arrives changes nothing about that.
#[test]
fn a_change_answered_during_a_title_poll_is_kept() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session_with_title_acquisition(
            SESSION,
            crate::TitleAcquisition::awaiting_first_prompt(/*list_supported*/ true),
        );
        let (mut turn, running) = driven.restored(model("b")).await;
        driven.end_turn(&running);
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
        let (choice, request) = driven.choose().await;
        // Stays unanswered, so the choice is answered while the poll runs.
        driven.asked("session/list").await;

        assert_eq!(driven.accept(choice, &request).await, Some("a".to_string()));

        assert_eq!(loaded(driven.load().await).await, on("a"));
        drop(turn);
        driven.fixture.shutdown().await;
    });
}

/// Neither does the reload that refreshes MCP, which reports no options of its own.
#[test]
fn a_change_answered_during_an_mcp_refresh_is_kept() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session_owing_mcp_refresh(SESSION);
        let admitted = driven.prompt();
        let refresh = driven.asked("session/load").await;
        let (choice, request) = driven.choose().await;

        assert_eq!(driven.accept(choice, &request).await, Some("a".to_string()));

        driven.answer(&refresh, json!({}));
        let mut turn = admitted.await.unwrap();
        let running = driven.asked("session/prompt").await;
        driven.end_turn(&running);
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
        assert_eq!(loaded(driven.load().await).await, on("a"));
        drop(turn);
        driven.fixture.shutdown().await;
    });
}

/// An answer for a provider session that was replaced meanwhile says nothing about its replacement.
/// The choice it answers is settled with what the replacement offers, without waiting for it.
#[test]
fn a_late_answer_does_not_describe_the_session_that_replaced_its_own() {
    for mid_turn in [true, false] {
        run_case(async move {
            let mut driven = Driven::new().await;
            driven.fixture.add_session(SESSION);
            // Without options there is no model to carry over to the replacement.
            let (mut turn, stale) = driven.restored(json!([])).await;
            let (choice, request) = driven.choose().await;
            driven.fail(&stale);
            let create = driven.asked("session/new").await;
            driven.answer(
                &create,
                json!({"sessionId":"replacement", "configOptions":model("b")}),
            );
            let running = driven.asked("session/prompt").await;
            assert_eq!(chosen(choice).await, Some("b".to_string()));

            if mid_turn {
                driven.answer_choice(&request);
            }
            driven.end_turn(&running);
            assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
            if !mid_turn {
                driven.answer_choice(&request);
            }

            assert_eq!(loaded(driven.load().await).await, on("b"));
            drop(turn);
            driven.fixture.shutdown().await;
        });
    }
}

/// Where the actor is when the agent's frames reach it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Reading {
    Turn,
    Idle,
    TitlePoll,
    McpRefresh,
}

/// The last frame the agent sent that carries options decides, wherever the frames are read.
///
/// Both are written before the actor is scheduled again, so it finds them ready together.
fn the_last_frame_read_decides(reading: Reading) {
    for (push_last, expected) in [(false, "a"), (true, "b")] {
        run_case(async move {
            let mut driven = Driven::new().await;
            match reading {
                Reading::McpRefresh => driven.fixture.add_session_owing_mcp_refresh(SESSION),
                Reading::TitlePoll => driven.fixture.add_session_with_title_acquisition(
                    SESSION,
                    crate::TitleAcquisition::awaiting_first_prompt(/*list_supported*/ true),
                ),
                Reading::Turn | Reading::Idle => driven.fixture.add_session(SESSION),
            }
            let mut refreshing = None;
            let mut running = None;
            if reading == Reading::McpRefresh {
                let admitted = driven.prompt();
                let refresh = driven.asked("session/load").await;
                refreshing = Some((admitted, refresh));
            } else {
                running = Some(driven.restored(model("b")).await);
            }
            if let Reading::Idle | Reading::TitlePoll = reading
                && let Some((turn, request)) = running.as_mut()
            {
                driven.end_turn(request);
                assert_eq!(completion(turn).await, StopReason::EndTurn);
            }
            let (choice, request) = driven.choose().await;
            if reading == Reading::TitlePoll {
                // Stays unanswered, so the frames arrive while the poll runs.
                driven.asked("session/list").await;
            }

            if push_last {
                driven.answer_choice(&request);
                driven.push("b");
            } else {
                driven.push("b");
                driven.answer_choice(&request);
            }
            choice.await.unwrap().unwrap();

            if let Some((admitted, refresh)) = refreshing {
                driven.answer(&refresh, json!({}));
                let turn = admitted.await.unwrap();
                let request = driven.asked("session/prompt").await;
                running = Some((turn, request));
            }
            if let Reading::Turn | Reading::McpRefresh = reading
                && let Some((turn, request)) = running.as_mut()
            {
                driven.end_turn(request);
                assert_eq!(completion(turn).await, StopReason::EndTurn);
            }
            assert_eq!(
                loaded(driven.load().await).await,
                on(expected),
                "{reading:?}"
            );
            drop(running);
            driven.fixture.shutdown().await;
        });
    }
}

#[test]
fn the_last_frame_a_running_turn_reads_decides_the_options() {
    the_last_frame_read_decides(Reading::Turn);
}

#[test]
fn the_last_frame_an_idle_session_reads_decides_the_options() {
    the_last_frame_read_decides(Reading::Idle);
}

#[test]
fn the_last_frame_a_title_poll_reads_decides_the_options() {
    the_last_frame_read_decides(Reading::TitlePoll);
}

#[test]
fn the_last_frame_an_mcp_refresh_reads_decides_the_options() {
    the_last_frame_read_decides(Reading::McpRefresh);
}

/// What a title poll read is not taken again when the next turn delivers what was kept for it.
#[test]
fn options_pushed_during_a_title_poll_do_not_return_with_the_next_turn() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session_with_title_acquisition(
            SESSION,
            crate::TitleAcquisition::awaiting_first_prompt(/*list_supported*/ true),
        );
        let (mut turn, running) = driven.restored(model("a")).await;
        driven.end_turn(&running);
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
        driven.asked("session/list").await;
        driven.push("b");
        // A command would end the poll, so it is given time to read the push instead.
        tokio::time::sleep(Duration::from_millis(1)).await;
        let (choice, request) = driven.choose().await;
        assert_eq!(driven.accept(choice, &request).await, Some("a".to_string()));

        let mut next = driven.prompt().await.unwrap();
        let running = driven.asked("session/prompt").await;
        driven.end_turn(&running);
        let mut told = Vec::new();
        while let Some(event) = next.recv().await {
            match event.unwrap() {
                PromptSessionEvent::SessionUpdate { update, .. } => told.extend(state(update)),
                PromptSessionEvent::Completed { .. } => break,
                unexpected => panic!("unexpected prompt event: {unexpected:?}"),
            }
        }

        assert_eq!(told, on("a"));
        assert_eq!(loaded(driven.load().await).await, on("a"));
        drop((turn, next));
        driven.fixture.shutdown().await;
    });
}

/// A reload that answers with options is as good a report as any other.
#[test]
fn options_an_mcp_refresh_answers_with_are_taken() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session_owing_mcp_refresh(SESSION);
        let admitted = driven.prompt();
        let refresh = driven.asked("session/load").await;
        driven.push("a");
        driven.answer(&refresh, json!({"configOptions":model("b")}));
        let mut turn = admitted.await.unwrap();
        let running = driven.asked("session/prompt").await;
        driven.end_turn(&running);
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);

        assert_eq!(loaded(driven.load().await).await, on("b"));
        drop(turn);
        driven.fixture.shutdown().await;
    });
}

/// A change the agent refuses fails and leaves the session on what it was.
#[test]
fn a_refused_change_leaves_the_options_alone() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session(SESSION);
        let (turn, _running) = driven.restored(model("b")).await;
        let (choice, request) = driven.choose().await;
        driven.fail(&request);

        assert!(choice.await.unwrap().is_err());
        let view = driven.load().await;
        driven.fixture.manager.stop_session(stop()).await.unwrap();
        assert_eq!(loaded(view).await, on("b"));
        drop(turn);
        driven.fixture.shutdown().await;
    });
}

/// A caller that gave up while the session was being restored never asked for anything.
#[test]
fn a_change_given_up_before_it_was_asked_is_not_asked() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session(SESSION);
        let admitted = driven.prompt();
        let restore = driven.asked("session/load").await;
        let manager = driven.fixture.manager.clone();
        let choice = tokio::spawn(async move { manager.set_session_config(change()).await });
        tokio::task::yield_now().await;
        choice.abort();
        assert!(choice.await.unwrap_err().is_cancelled());
        driven.answer(&restore, json!({"configOptions":model("b")}));
        let mut turn = admitted.await.unwrap();

        let running = driven.asked("session/prompt").await;
        driven.end_turn(&running);
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
        // The actor has read every command by now.
        loaded(driven.load().await).await;
        while let Ok(frame) = driven.fixture.frames.try_recv() {
            assert_ne!(frame["method"], "session/set_config_option");
        }
        drop(turn);
        driven.fixture.shutdown().await;
    });
}

/// What an earlier attachment reported does not stand in for a session that reports nothing.
#[test]
fn a_session_restored_without_options_is_on_none() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session(SESSION);
        let (mut turn, running) = driven.restored(model("b")).await;
        driven.end_turn(&running);
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
        driven.fixture.manager.stop_session(stop()).await.unwrap();

        let admitted = driven.prompt();
        let restore = driven.asked("session/load").await;
        driven.answer(&restore, json!({}));
        let mut again = admitted.await.unwrap();
        let running = driven.asked("session/prompt").await;
        driven.end_turn(&running);
        assert_eq!(completion(&mut again).await, StopReason::EndTurn);

        assert_eq!(loaded(driven.load().await).await, Vec::new());
        drop((turn, again));
        driven.fixture.shutdown().await;
    });
}

/// Changes do not wait for each other; the order of their answers decides.
#[test]
fn changes_in_flight_are_answered_in_any_order() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session(SESSION);
        let (mut turn, running) = driven.restored(model("b")).await;
        let (first, first_request) = driven.choose().await;
        let (second, second_request) = driven.choose().await;
        driven.answer(&second_request, json!({"configOptions":model("a")}));
        assert_eq!(chosen(second).await, Some("a".to_string()));
        driven.answer(&first_request, json!({"configOptions":model("b")}));
        assert_eq!(chosen(first).await, Some("b".to_string()));
        driven.end_turn(&running);
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);

        assert_eq!(loaded(driven.load().await).await, on("b"));
        drop(turn);
        driven.fixture.shutdown().await;
    });
}

/// A change its caller gave up on is still the agent's to make.
#[test]
fn an_answer_after_the_caller_timed_out_is_taken() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session(SESSION);
        let (mut turn, running) = driven.restored(model("b")).await;
        driven.end_turn(&running);
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
        let (choice, request) = driven.choose().await;
        // The paused clock advances to the timeout while nothing else is left to do.
        let error = choice.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");

        driven.answer_choice(&request);
        while loaded(driven.load().await).await != on("a") {}
        drop(turn);
        driven.fixture.shutdown().await;
    });
}

/// A turn that is being cancelled still reads the answers of others.
#[test]
fn a_change_answered_while_a_turn_is_cancelled_is_kept() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session(SESSION);
        let (mut turn, running) = driven.restored(model("b")).await;
        let (choice, request) = driven.choose().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        driven
            .fixture
            .manager
            .cancel_turn_prompt(SESSION, deadline)
            .unwrap();
        driven.asked("session/cancel").await;

        assert_eq!(driven.accept(choice, &request).await, Some("a".to_string()));
        driven.answer(&running, json!(PromptResponse::new(StopReason::Cancelled)));
        assert_eq!(completion(&mut turn).await, StopReason::Cancelled);
        assert_eq!(loaded(driven.load().await).await, on("a"));
        drop(turn);
        driven.fixture.shutdown().await;
    });
}

/// So does one whose own answer is already there when the cancel is about to be written.
#[test]
fn a_change_answered_just_before_a_cancelled_turn_is_kept() {
    // Which of the ready inputs the turn reads first is not fixed.
    for _ in 0..16 {
        run_case(async {
            let mut driven = Driven::new().await;
            driven.fixture.add_session(SESSION);
            let (mut turn, running) = driven.restored(model("b")).await;
            let (choice, request) = driven.choose().await;
            driven.answer_choice(&request);
            driven.end_turn(&running);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
            driven
                .fixture
                .manager
                .cancel_turn_prompt(SESSION, deadline)
                .unwrap();
            assert_eq!(chosen(choice).await, Some("a".to_string()));
            completion(&mut turn).await;
            assert_eq!(loaded(driven.load().await).await, on("a"));
            drop(turn);
            driven.fixture.shutdown().await;
        });
    }
}

/// How a provider session goes while a change is on its way to it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Released {
    Stopped,
    StoppedMidTurn,
    Deleted,
    Switched,
    /// The same, while a title poll or an MCP refresh borrows the session.
    StoppedInTitlePoll,
    StoppedInMcpRefresh,
    /// The turn's owner stopped reading its output.
    Unread,
    /// It reads that, but has no room for the turn's end.
    EndUnread,
    Overflowed,
    Disconnected,
    DisconnectedMidTurn,
}

/// A session the host gives up was still asked for the change; one lost to its agent is another
/// matter. Nobody waits for an answer that can no longer arrive.
#[test]
fn a_change_in_flight_is_settled_when_its_session_goes() {
    for (released, succeeds) in [
        (Released::Stopped, true),
        (Released::StoppedMidTurn, true),
        (Released::Deleted, true),
        (Released::Switched, true),
        (Released::StoppedInTitlePoll, true),
        (Released::StoppedInMcpRefresh, true),
        (Released::Unread, true),
        (Released::EndUnread, true),
        (Released::Overflowed, false),
        (Released::Disconnected, false),
        (Released::DisconnectedMidTurn, false),
    ] {
        run_case(async move {
            let mut driven = Driven::new().await;
            let mut turn = None;
            let mut ending = None;
            if released == Released::Switched {
                driven
                    .fixture
                    .add_session_bound_elsewhere(SESSION, "official/other.agent");
            } else if released == Released::StoppedInMcpRefresh {
                driven.fixture.add_session_owing_mcp_refresh(SESSION);
                // No prompt is being admitted, which would hold back the stop.
                let session = ora_domain::SessionId::new(SESSION);
                let actor = driven.fixture.manager.lookup_actor(&session).unwrap();
                actor
                    .unwrap()
                    .commands
                    .send(crate::command::RuntimeCommand::McpDesiredMaybeChanged)
                    .unwrap();
                driven.asked("session/load").await;
            } else {
                if released == Released::StoppedInTitlePoll {
                    driven.fixture.add_session_with_title_acquisition(
                        SESSION,
                        crate::TitleAcquisition::awaiting_first_prompt(
                            /*list_supported*/ true,
                        ),
                    );
                } else {
                    driven.fixture.add_session(SESSION);
                }
                let (mut started, running) = driven.restored(model("b")).await;
                if let Released::StoppedInTitlePoll
                | Released::Stopped
                | Released::Deleted
                | Released::Overflowed
                | Released::Disconnected = released
                {
                    driven.end_turn(&running);
                    assert_eq!(completion(&mut started).await, StopReason::EndTurn);
                }
                turn = Some(started);
                ending = Some(running);
            }
            let (choice, _unanswered) = driven.choose().await;
            if released == Released::StoppedInTitlePoll {
                driven.asked("session/list").await;
            }
            let asked = tokio::time::Instant::now();

            let manager = driven.fixture.manager.clone();
            let routes = driven.fixture.supervisor.routes.clone();
            match released {
                Released::Stopped
                | Released::StoppedMidTurn
                | Released::StoppedInTitlePoll
                | Released::StoppedInMcpRefresh => {
                    manager.stop_session(stop()).await.unwrap();
                }
                Released::Deleted => {
                    manager.delete_session(SESSION).await.unwrap();
                }
                Released::Switched => {
                    let switch = tokio::spawn(async move {
                        manager
                            .switch_agent(ora_contracts::SwitchSessionAgentRequest {
                                session_id: SESSION.into(),
                                agent_ref: "official/test.agent".into(),
                                model: None,
                            })
                            .await
                            .unwrap()
                    });
                    let create = driven.asked("session/new").await;
                    driven.answer(
                        &create,
                        json!({"sessionId":"moved", "configOptions":model("b")}),
                    );
                    switch.await.unwrap();
                }
                Released::Unread => {
                    for _ in 0..2 {
                        for _ in 0..crate::CONTRACT_QUEUE_CAPACITY / 2 + 1 {
                            routes.route_event(chunk()).unwrap();
                        }
                        // The turn forwards what it read before the next half arrives.
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
                Released::EndUnread => {
                    // The options the turn started with take the remaining place.
                    let room = crate::CONTRACT_QUEUE_CAPACITY - 1;
                    for part in [room / 2, room - room / 2] {
                        for _ in 0..part {
                            routes.route_event(chunk()).unwrap();
                        }
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                    assert!(!choice.is_finished(), "the turn still has its owner");
                    driven.end_turn(ending.as_ref().unwrap());
                }
                Released::Overflowed => {
                    for _ in 0..=crate::CONTRACT_QUEUE_CAPACITY {
                        let _ = routes.route_event(chunk());
                    }
                }
                Released::Disconnected | Released::DisconnectedMidTurn => {
                    routes.fail_generation(1, crate::support::runtime_unavailable());
                }
            }

            let outcome = choice.await.unwrap();
            assert_eq!(outcome.is_ok(), succeeds, "{released:?}: {outcome:?}");
            assert!(
                asked.elapsed() < crate::SESSION_SETUP_TIMEOUT,
                "{released:?} waited for the answer"
            );
            drop(turn);
            driven.fixture.shutdown().await;
        });
    }
}

/// Chooses model `a`.
fn change() -> SetSessionConfigRequest {
    SetSessionConfigRequest {
        session_id: SESSION.into(),
        config_id: "model".into(),
        value: "a".into(),
    }
}

/// Names the session to stop.
fn stop() -> StopSessionRequest {
    StopSessionRequest {
        session_id: SESSION.into(),
    }
}

/// Builds one piece of a turn's output.
fn chunk() -> crate::routing::SessionEvent {
    crate::routing::SessionEvent::update(
        agent_client_protocol_schema::v1::SessionNotification::new(
            SESSION,
            SessionUpdate::AgentMessageChunk(agent_client_protocol_schema::v1::ContentChunk::new(
                "output".into(),
            )),
        ),
    )
}
