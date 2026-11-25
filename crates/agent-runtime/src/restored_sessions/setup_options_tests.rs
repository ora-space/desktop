//! Covers which model the views of a new provider session end up on when the agent pushes its own
//! configuration while Ora applies the one the user chose.

use super::follower_setup_tests::{State, model, options, state};
use super::*;
use agent_client_protocol_schema::v1::{ConfigOptionUpdate, CurrentModeUpdate};
use ora_contracts::{LoadSessionEvent, LoadSessionRequest, StartSessionRequest};
use pretty_assertions::assert_eq;

/// What the agent does with the user's model.
#[derive(Clone, Copy)]
enum Intent {
    /// It applies model `a`.
    Applied,
    /// The same, after pushing a last snapshot on model `b`.
    AppliedAfterPush,
    /// It applies model `a` too late for setup, with the first prompt.
    AppliedLate,
    /// It offers model `a` and then refuses it.
    Refused,
    /// The same, after moving to model `a` on its own.
    RefusedAfterPush,
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
    /// Restored by the prompt, which carries the model.
    Restored,
}

/// Frames a mode change, which applying a model says nothing about.
fn mode(session_id: &str) -> Value {
    let update = CurrentModeUpdate::new("plan");
    json!({"jsonrpc":"2.0", "method":"session/update", "params": SessionNotification::new(
        session_id.to_string(), SessionUpdate::CurrentModeUpdate(update))})
}

/// Answers setup on model `b`, pushing that snapshot and a mode right after it.
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
    let mut late = Vec::new();
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
                mode("created"),
            ],
            Some("session/load") => vec![answer(json!({"configOptions":model("b")}))],
            Some("session/set_config_option") => match intent {
                Intent::AppliedLate => {
                    late = vec![answer(json!({"configOptions":model("a")}))];
                    Vec::new()
                }
                Intent::Refused | Intent::RefusedAfterPush => {
                    let session = frame["params"]["sessionId"].as_str().unwrap();
                    let push =
                        matches!(intent, Intent::RefusedAfterPush).then(|| options(session, "a"));
                    let refusal = json!({"jsonrpc":"2.0", "id":id,
                        "error":{"code":-32000, "message":"refused"}});
                    push.into_iter().chain([refusal]).collect()
                }
                Intent::AppliedAfterPush => {
                    let session = frame["params"]["sessionId"].as_str().unwrap();
                    vec![
                        options(session, "b"),
                        answer(json!({"configOptions":model("a")})),
                    ]
                }
                Intent::Applied | Intent::NotOffered | Intent::Absent => {
                    vec![answer(json!({"configOptions":model("a")}))]
                }
            },
            Some("session/prompt") => std::mem::take(&mut late)
                .into_iter()
                .chain([answer(json!(PromptResponse::new(StopReason::EndTurn)))])
                .collect(),
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
        Intent::Applied
        | Intent::AppliedAfterPush
        | Intent::AppliedLate
        | Intent::Refused
        | Intent::RefusedAfterPush => Some("a".to_string()),
        Intent::NotOffered => Some("z".to_string()),
        Intent::Absent => None,
    };
    let mut request_model = None;
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
        Acquired::Restored => {
            fixture.add_session("restored");
            request_model = model;
            ("restored".to_string(), Vec::new())
        }
    };
    let setup = state(SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
        config_options,
    )));
    let mut turn = fixture
        .manager
        .prompt_session(PromptSessionRequest {
            model: request_model,
            ..prompt(&session_id)
        })
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
/// must not undo it. Without such an answer the snapshot is the newest thing the agent said. Either
/// way setup reports it, so that the first prompt has no options left to deliver.
#[test]
fn a_snapshot_older_than_the_applied_model_does_not_undo_it() {
    let on = |model: &str| vec![State::Model(model.to_string())];
    // What else was collected before the model was applied is as current as ever.
    let mode = || State::Mode("plan".to_string());
    for acquired in [Acquired::Started, Acquired::Switched] {
        for (intent, expected) in [
            (Intent::Applied, (on("a"), vec![mode()], on("a"))),
            (Intent::AppliedAfterPush, (on("a"), vec![mode()], on("a"))),
            (Intent::AppliedLate, (on("b"), vec![mode()], on("a"))),
            (Intent::Refused, (on("b"), vec![mode()], on("b"))),
            (Intent::RefusedAfterPush, (on("a"), vec![mode()], on("a"))),
            (Intent::NotOffered, (on("b"), vec![mode()], on("b"))),
            (Intent::Absent, (on("b"), vec![mode()], on("b"))),
        ] {
            run_case(async move {
                assert_eq!(views(acquired, intent).await, expected);
            });
        }
    }
}

/// A restored session keeps the options it was restored with unless it answered with others.
#[test]
fn a_restored_session_is_on_the_model_its_agent_accepted() {
    let on = |model: &str| vec![State::Model(model.to_string())];
    for (intent, prompted, expected) in [
        (Intent::Applied, "a", "a"),
        (Intent::AppliedAfterPush, "a", "a"),
        (Intent::AppliedLate, "b", "a"),
        (Intent::Refused, "b", "b"),
        (Intent::RefusedAfterPush, "a", "a"),
        (Intent::NotOffered, "b", "b"),
        (Intent::Absent, "b", "b"),
    ] {
        run_case(async move {
            assert_eq!(
                views(Acquired::Restored, intent).await,
                (vec![State::NoOptions], on(prompted), on(expected))
            );
        });
    }
}
