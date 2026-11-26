//! Covers what a load follower learns about the provider session that replaces a stale one.
//!
//! A follower joined on the restored session, so what that one reported is on its screen; the
//! replacement reports its own options and commands once, during its setup.

use super::*;
use agent_client_protocol_schema::v1::{
    AvailableCommand, AvailableCommandsUpdate, ConfigOptionUpdate,
};
use ora_contracts::{LoadSessionEvent, LoadSessionRequest};
use pretty_assertions::assert_eq;

/// When the replacement reports its commands.
#[derive(Clone, Copy)]
enum Report {
    /// Within the window that collects `session/new` commands.
    InSetup,
    /// After it, while Ora applies the model the conversation was on.
    DuringModel,
    /// Only once setup is over.
    AfterSetup,
    Never,
    /// Never, after a stale session that had reported none either.
    NeverAny,
    /// Never, and neither session offers options.
    Nothing,
    /// Only the agent's own options, pushed before Ora applies the conversation's model.
    OptionsInSetup,
    /// The same, and then it refuses that model.
    OptionsKept,
    /// Neither commands nor options: the replacement offers no options at all.
    NoOptions,
}

/// The session state a client shows, as the updates that carry it.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum State {
    Commands(Vec<String>),
    Model(String),
    NoOptions,
    Mode(String),
}

/// Reads one update's session state, if it carries any.
pub(super) fn state(update: SessionUpdate) -> Option<State> {
    match update {
        SessionUpdate::AvailableCommandsUpdate(update) => Some(State::Commands(
            update
                .available_commands
                .into_iter()
                .map(|command| command.name)
                .collect(),
        )),
        SessionUpdate::ConfigOptionUpdate(update) if update.config_options.is_empty() => {
            Some(State::NoOptions)
        }
        SessionUpdate::ConfigOptionUpdate(update) => Some(State::Model(
            crate::model_config::current_model(&update.config_options).unwrap_or_default(),
        )),
        SessionUpdate::CurrentModeUpdate(update) => {
            Some(State::Mode(update.current_mode_id.to_string()))
        }
        _ => None,
    }
}

/// Builds the model selector as one provider session reports it.
pub(super) fn model(current: &str) -> Value {
    json!([{"id":"model", "name":"Model", "category":"model", "type":"select",
        "currentValue":current, "options":[{"value":"a","name":"A"},{"value":"b","name":"B"}]}])
}

/// Frames a catalog of one command.
fn commands(session_id: &str, name: &str) -> Value {
    let update = AvailableCommandsUpdate::new(vec![AvailableCommand::new(name, "Runs it")]);
    json!({"jsonrpc":"2.0", "method":"session/update", "params": SessionNotification::new(
        session_id.to_string(), SessionUpdate::AvailableCommandsUpdate(update))})
}

/// Frames an option change the provider pushed on its own.
pub(super) fn options(session_id: &str, current: &str) -> Value {
    let update = ConfigOptionUpdate::new(serde_json::from_value(model(current)).unwrap());
    json!({"jsonrpc":"2.0", "method":"session/update", "params": SessionNotification::new(
        session_id.to_string(), SessionUpdate::ConfigOptionUpdate(update))})
}

/// Restores a session that fails its first prompt, then replaces it as scripted.
async fn replacing_agent(
    report: Report,
    fail: Arc<tokio::sync::Notify>,
    mut reader: DuplexStream,
    mut writer: DuplexStream,
    frames: mpsc::UnboundedSender<Value>,
    exited: watch::Sender<bool>,
) {
    let registration = json!({"jsonrpc":"2.0","method":"ora/register",
        "params":{"methods":[],"emits":["agent/acp"]}});
    write_message(&mut writer, &registration).await.unwrap();
    while let Ok(Some(envelope)) = read_message(&mut reader).await {
        if envelope["method"] == "ora/shutdown" {
            break;
        }
        let frame = envelope["params"].clone();
        frames.send(frame.clone()).unwrap();
        let id = &frame["id"];
        let answer = |result: Value| json!({"jsonrpc":"2.0", "id":id, "result":result});
        let replies = match frame["method"].as_str() {
            Some("session/load") => {
                let offered = match report {
                    Report::Nothing => json!([]),
                    _ => model("a"),
                };
                let restored = answer(json!({"configOptions":offered}));
                match report {
                    Report::NeverAny | Report::Nothing => vec![restored],
                    _ => vec![commands("restored", "stale"), restored],
                }
            }
            Some("session/prompt") if frame["params"]["sessionId"] == "restored" => {
                fail.notified().await;
                vec![json!({"jsonrpc":"2.0", "id":id,
                    "error":{"code":-32000, "message":"stale session"}})]
            }
            Some("session/new") => {
                let offered = match report {
                    Report::NoOptions | Report::Nothing => json!([]),
                    _ => model("b"),
                };
                let created = answer(json!({"sessionId":"replacement", "configOptions":offered}));
                match report {
                    Report::InSetup => vec![created, commands("replacement", "fresh")],
                    Report::OptionsInSetup | Report::OptionsKept => {
                        vec![created, options("replacement", "b")]
                    }
                    Report::DuringModel
                    | Report::AfterSetup
                    | Report::Never
                    | Report::NeverAny
                    | Report::Nothing
                    | Report::NoOptions => {
                        vec![created]
                    }
                }
            }
            Some("session/set_config_option") => {
                let applied = answer(json!({"configOptions":model("a")}));
                match report {
                    Report::OptionsKept => vec![json!({"jsonrpc":"2.0", "id":id,
                        "error":{"code":-32000, "message":"refused"}})],
                    Report::DuringModel => vec![commands("replacement", "fresh"), applied],
                    Report::AfterSetup => vec![applied, commands("replacement", "fresh")],
                    Report::InSetup
                    | Report::Never
                    | Report::NeverAny
                    | Report::Nothing
                    | Report::OptionsInSetup
                    | Report::NoOptions => vec![applied],
                }
            }
            Some("session/prompt") => {
                vec![answer(json!(PromptResponse::new(StopReason::EndTurn)))]
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

/// Runs the replaced turn and returns the session state its owner and a follower were given.
async fn replaced_turn(report: Report) -> (Vec<State>, Vec<State>) {
    let fail = Arc::new(tokio::sync::Notify::new());
    let gate = fail.clone();
    let mut fixture = ReviewDrainFixture::new_with(move |reader, writer, _, frames, exited| {
        replacing_agent(report, gate, reader, writer, frames, exited)
    })
    .await;
    fixture.add_session("restored");
    let mut owner = fixture
        .manager
        .prompt_session(prompt("restored"))
        .await
        .unwrap();
    while fixture.next_frame().await["method"] != "session/prompt" {}
    let mut follower = fixture
        .manager
        .load_session(LoadSessionRequest {
            session_id: "restored".into(),
        })
        .await
        .unwrap();
    fail.notify_one();

    let mut owned = Vec::new();
    while let Some(event) = owner.recv().await {
        match event.unwrap() {
            PromptSessionEvent::SessionUpdate { update, .. } => owned.extend(state(update)),
            PromptSessionEvent::Completed { .. } => break,
            unexpected => panic!("unexpected prompt event: {unexpected:?}"),
        }
    }
    let mut followed = Vec::new();
    while let Some(event) = follower.recv().await {
        if let LoadSessionEvent::SessionUpdate { update, .. } = event.unwrap() {
            followed.extend(state(update));
        }
    }
    drop((owner, follower));
    fixture.shutdown().await;
    (owned, followed)
}

/// Both views end up on what the replacement reported, whenever its commands arrived.
#[test]
fn a_follower_sees_the_state_of_the_session_that_replaced_a_stale_one() {
    let stale = || State::Commands(vec!["stale".to_string()]);
    let fresh = || State::Commands(vec!["fresh".to_string()]);
    let none = || State::Commands(Vec::new());
    let on = |model: &str| State::Model(model.to_string());
    for (report, replaced) in [
        (Report::InSetup, vec![on("a"), fresh()]),
        (Report::DuringModel, vec![on("a"), fresh()]),
        (Report::AfterSetup, vec![on("a"), none(), fresh()]),
        (Report::Never, vec![on("a"), none()]),
        (Report::OptionsInSetup, vec![on("a"), none()]),
        (Report::OptionsKept, vec![on("b"), none()]),
        (Report::NoOptions, vec![State::NoOptions, none()]),
    ] {
        run_case(async move {
            let (owned, followed) = replaced_turn(report).await;
            // A follower is first told what it joined on; the rest is the replacement's.
            assert_eq!(followed[..2], [on("a"), stale()]);
            assert_eq!(followed[2..], replaced);
            assert_eq!(owned[owned.len() - replaced.len()..], replaced);
        });
    }
}

/// Nothing is withdrawn that was never reported: an empty report would say the agent has none.
#[test]
fn a_replacement_withdraws_nothing_the_stale_session_never_reported() {
    let on_a = || State::Model("a".to_string());
    for (report, expected) in [
        (Report::NeverAny, vec![on_a(), on_a()]),
        (Report::Nothing, Vec::new()),
    ] {
        run_case(async move {
            assert_eq!(replaced_turn(report).await, (expected.clone(), expected));
        });
    }
}
