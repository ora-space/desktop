//! Covers what a view is told after the user configured a session whose agent says what changed only
//! in its answer.

use super::follower_setup_tests::{State, model, state};
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
    async fn choose(&mut self) -> (tokio::task::JoinHandle<SetSessionConfigResponse>, Value) {
        let manager = self.fixture.manager.clone();
        let choice = tokio::spawn(async move {
            manager
                .set_session_config(SetSessionConfigRequest {
                    session_id: SESSION.into(),
                    config_id: "model".into(),
                    value: "a".into(),
                })
                .await
                .unwrap()
        });
        let request = self.asked("session/set_config_option").await;
        (choice, request)
    }

    /// Has the agent answer the choice with model `a`, and returns what its caller is told.
    async fn accept(
        &self,
        choice: tokio::task::JoinHandle<SetSessionConfigResponse>,
        request: &Value,
    ) -> Option<String> {
        self.answer(request, json!({"configOptions":model("a")}));
        crate::model_config::current_model(&choice.await.unwrap().config_options)
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

/// An answer for a provider session that was replaced meanwhile says nothing about its replacement,
/// to later views and to whoever made the choice alike.
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
            let told = if mid_turn {
                let told = driven.accept(choice, &request).await;
                driven.end_turn(&running);
                assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
                told
            } else {
                driven.end_turn(&running);
                assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
                driven.accept(choice, &request).await
            };

            assert_eq!(told, Some("b".to_string()));
            assert_eq!(loaded(driven.load().await).await, on("b"));
            drop(turn);
            driven.fixture.shutdown().await;
        });
    }
}

/// The agent made the change, so a session that stopped meanwhile does not turn it into a failure.
#[test]
fn a_change_answered_after_the_session_stopped_still_succeeds() {
    run_case(async {
        let mut driven = Driven::new().await;
        driven.fixture.add_session(SESSION);
        let (mut turn, running) = driven.restored(model("b")).await;
        driven.end_turn(&running);
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
        let (choice, request) = driven.choose().await;
        driven
            .fixture
            .manager
            .stop_session(StopSessionRequest {
                session_id: SESSION.into(),
            })
            .await
            .unwrap();

        assert_eq!(driven.accept(choice, &request).await, Some("a".to_string()));
        drop(turn);
        driven.fixture.shutdown().await;
    });
}
