//! Covers which model the views of a new provider session end up on when the agent pushes its own
//! configuration while Ora applies the one the user chose.

use super::follower_setup_tests::{State, model, options, state};
use super::*;
use ora_contracts::{LoadSessionEvent, LoadSessionRequest, StartSessionRequest};
use pretty_assertions::assert_eq;

/// What the agent does with the user's model.
#[derive(Clone, Copy)]
enum Intent {
    /// It applies model `a`.
    Applied,
    /// It offers model `a` and then refuses it.
    Refused,
    /// The user asked for a model it does not offer.
    NotOffered,
    /// The user asked for none.
    Absent,
}

/// The conversation that acquires the provider session.
#[derive(Clone, Copy)]
enum Acquired {
    Started,
    Switched,
}

/// Answers setup on model `b`, pushing that snapshot right after it.
async fn pushing_agent(
    intent: Intent,
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
            Some("session/new") => vec![
                answer(json!({"sessionId":"created", "configOptions":model("b")})),
                options("created", "b"),
            ],
            Some("session/set_config_option") => match intent {
                Intent::Refused => vec![json!({"jsonrpc":"2.0", "id":id,
                    "error":{"code":-32000, "message":"refused"}})],
                Intent::Applied | Intent::NotOffered | Intent::Absent => {
                    vec![answer(json!({"configOptions":model("a")}))]
                }
            },
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

/// Returns the state setup answered with, then what the first prompt and a later load add to it.
async fn views(acquired: Acquired, intent: Intent) -> (Vec<State>, Vec<State>, Vec<State>) {
    let fixture = ReviewDrainFixture::new_with(move |reader, writer, _, frames, exited| {
        pushing_agent(intent, reader, writer, frames, exited)
    })
    .await;
    let model = match intent {
        Intent::Applied | Intent::Refused => Some("a".to_string()),
        Intent::NotOffered => Some("z".to_string()),
        Intent::Absent => None,
    };
    let (session_id, config_options) = match acquired {
        Acquired::Started => {
            let started = fixture
                .manager
                .start_session(StartSessionRequest {
                    workspace_id: "workspace".into(),
                    agent_ref: "official/test.agent".into(),
                    model,
                })
                .await
                .unwrap();
            (started.session.id, started.config_options)
        }
        Acquired::Switched => {
            fixture.seed_session_on("moving", "official/other.agent");
            let switched = fixture
                .manager
                .switch_agent(ora_contracts::SwitchSessionAgentRequest {
                    session_id: "moving".into(),
                    agent_ref: "official/test.agent".into(),
                    model,
                })
                .await
                .unwrap();
            (switched.session.id, switched.config_options)
        }
    };
    let setup = state(SessionUpdate::ConfigOptionUpdate(
        agent_client_protocol_schema::v1::ConfigOptionUpdate::new(config_options),
    ));
    let mut turn = fixture
        .manager
        .prompt_session(prompt(&session_id))
        .await
        .unwrap();
    let mut prompted = Vec::new();
    while let Some(event) = turn.recv().await {
        match event.unwrap() {
            PromptSessionEvent::SessionUpdate { update, .. } => prompted.extend(state(update)),
            PromptSessionEvent::Completed { .. } => break,
            unexpected => panic!("unexpected prompt event: {unexpected:?}"),
        }
    }
    let mut load = fixture
        .manager
        .load_session(LoadSessionRequest {
            session_id: session_id.clone(),
        })
        .await
        .unwrap();
    let mut loaded = Vec::new();
    while let Some(event) = load.recv().await {
        match event.unwrap() {
            LoadSessionEvent::SessionUpdate { update, .. } => loaded.extend(state(update)),
            LoadSessionEvent::Completed { .. } => break,
            _ => {}
        }
    }
    drop((turn, load));
    fixture.shutdown().await;
    (setup.into_iter().collect(), prompted, loaded)
}

/// A snapshot pushed before the model was applied is outdated by the answer that applied it, and
/// must not undo it. Without such an answer the snapshot is the newest thing the agent said.
#[test]
fn a_snapshot_older_than_the_applied_model_does_not_undo_it() {
    let on = |model: &str| vec![State::Model(model.to_string())];
    for acquired in [Acquired::Started, Acquired::Switched] {
        for (intent, expected) in [
            (Intent::Applied, (on("a"), Vec::new(), on("a"))),
            (Intent::Refused, (on("b"), on("b"), on("b"))),
            (Intent::NotOffered, (on("b"), on("b"), on("b"))),
            (Intent::Absent, (on("b"), on("b"), on("b"))),
        ] {
            run_case(async move {
                assert_eq!(views(acquired, intent).await, expected);
            });
        }
    }
}
