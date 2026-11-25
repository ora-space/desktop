//! Covers what a view is told after the user configured a session whose agent says what changed only
//! in its answer.

use super::follower_setup_tests::{State, model, state};
use super::*;
use ora_contracts::{LoadSessionEvent, LoadSessionRequest, SetSessionConfigRequest};
use pretty_assertions::assert_eq;

/// Restores on model `b`, or on no options at all, and never pushes a change.
///
/// With no options the restored session fails its first prompt. The answer to its configuration
/// call is then held until the replacement, which offers model `b`, is prompted.
async fn answering_agent(
    replaced: bool,
    finish: Arc<tokio::sync::Notify>,
    mut reader: DuplexStream,
    mut writer: DuplexStream,
    frames: mpsc::UnboundedSender<Value>,
    exited: watch::Sender<bool>,
) {
    let registration = json!({"jsonrpc":"2.0","method":"ora/register",
        "params":{"methods":[],"emits":["agent/acp"]}});
    write_message(&mut writer, &registration).await.unwrap();
    let mut prompted = None;
    let mut held = None;
    loop {
        let envelope = tokio::select! {
            envelope = read_message(&mut reader) => envelope,
            () = finish.notified() => {
                let answer = if replaced {
                    json!({"jsonrpc":"2.0", "id":prompted.take().expect("a prompt"),
                        "error":{"code":-32000, "message":"stale session"}})
                } else {
                    json!({"jsonrpc":"2.0", "id":prompted.take().expect("a prompt"),
                        "result":PromptResponse::new(StopReason::EndTurn)})
                };
                let envelope = json!({"jsonrpc":"2.0", "method":"agent/acp", "params":answer});
                write_message(&mut writer, &envelope).await.unwrap();
                continue;
            }
        };
        let Ok(Some(envelope)) = envelope else { break };
        if envelope["method"] == "ora/shutdown" {
            break;
        }
        let frame = envelope["params"].clone();
        frames.send(frame.clone()).unwrap();
        let id = frame["id"].clone();
        let answer = |id: &Value, result: Value| json!({"jsonrpc":"2.0", "id":id, "result":result});
        let replies = match frame["method"].as_str() {
            Some("session/load") => {
                let offered = if replaced { json!([]) } else { model("b") };
                vec![answer(&id, json!({"configOptions":offered}))]
            }
            Some("session/new") => vec![answer(
                &id,
                json!({"sessionId":"replacement", "configOptions":model("b")}),
            )],
            Some("session/set_config_option") if replaced => {
                held = Some(id);
                Vec::new()
            }
            Some("session/set_config_option") => {
                vec![answer(&id, json!({"configOptions":model("a")}))]
            }
            Some("session/prompt") if frame["params"]["sessionId"] == "replacement" => vec![
                answer(
                    &held.take().expect("a configuration call"),
                    json!({"configOptions":model("a")}),
                ),
                answer(&id, json!(PromptResponse::new(StopReason::EndTurn))),
            ],
            Some("session/prompt") => {
                prompted = Some(id);
                Vec::new()
            }
            _ => continue,
        };
        for reply in replies {
            let envelope = json!({"jsonrpc":"2.0", "method":"agent/acp", "params":reply});
            write_message(&mut writer, &envelope).await.unwrap();
        }
    }
    exited.send_replace(true);
}

/// Opens the session and returns its stream.
async fn load(fixture: &ReviewDrainFixture) -> SessionEventStream<LoadSessionEvent> {
    fixture
        .manager
        .load_session(LoadSessionRequest {
            session_id: "restored".into(),
        })
        .await
        .unwrap()
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

/// Chooses model `a`.
fn choice() -> SetSessionConfigRequest {
    SetSessionConfigRequest {
        session_id: "restored".into(),
        config_id: "model".into(),
        value: "a".into(),
    }
}

/// A view that opens the session learns of the change, whether or not a turn is running.
#[test]
fn a_view_opened_after_a_change_is_told_about_it() {
    for mid_turn in [true, false] {
        run_case(async move {
            let finish = Arc::new(tokio::sync::Notify::new());
            let gate = finish.clone();
            let mut fixture =
                ReviewDrainFixture::new_with(move |reader, writer, _, frames, exited| {
                    answering_agent(
                        /*replaced*/ false, gate, reader, writer, frames, exited,
                    )
                })
                .await;
            fixture.add_session("restored");
            let mut turn = fixture
                .manager
                .prompt_session(prompt("restored"))
                .await
                .unwrap();
            while fixture.next_frame().await["method"] != "session/prompt" {}
            if !mid_turn {
                finish.notify_one();
                assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
            }

            let answered = fixture.manager.set_session_config(choice()).await.unwrap();
            let view = load(&fixture).await;
            if mid_turn {
                finish.notify_one();
                assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
            }

            let on_a = State::Model("a".to_string());
            assert_eq!(
                crate::model_config::current_model(&answered.config_options),
                Some("a".to_string())
            );
            assert_eq!(loaded(view).await, vec![on_a]);
            drop(turn);
            fixture.shutdown().await;
        });
    }
}

/// An answer for a provider session that was replaced meanwhile says nothing about its replacement.
#[test]
fn a_late_answer_does_not_describe_the_session_that_replaced_its_own() {
    run_case(async {
        let finish = Arc::new(tokio::sync::Notify::new());
        let gate = finish.clone();
        let mut fixture = ReviewDrainFixture::new_with(move |reader, writer, _, frames, exited| {
            answering_agent(/*replaced*/ true, gate, reader, writer, frames, exited)
        })
        .await;
        fixture.add_session("restored");
        let mut turn = fixture
            .manager
            .prompt_session(prompt("restored"))
            .await
            .unwrap();
        while fixture.next_frame().await["method"] != "session/prompt" {}
        let manager = fixture.manager.clone();
        let setting = tokio::spawn(async move { manager.set_session_config(choice()).await });
        while fixture.next_frame().await["method"] != "session/set_config_option" {}
        finish.notify_one();
        assert_eq!(completion(&mut turn).await, StopReason::EndTurn);
        setting.await.unwrap().unwrap();

        assert_eq!(
            loaded(load(&fixture).await).await,
            vec![State::Model("b".to_string())]
        );
        drop(turn);
        fixture.shutdown().await;
    });
}
