//! Covers the slash-command catalog an Agent reports outside any prompt.
//!
//! Agents send `available_commands_update` once per setup, typically just after answering it, so a
//! catalog the runtime loses is never reported again for that session.

use super::*;
use agent_client_protocol_schema::v1::{
    AvailableCommand, AvailableCommandsUpdate, ContentChunk, Plan,
};
use ora_contracts::{LoadSessionEvent, LoadSessionRequest, StartSessionRequest};
use pretty_assertions::assert_eq;

/// Where in the restore handshake the Agent reports its catalog.
#[derive(Clone, Copy)]
enum Report {
    /// Only when the test injects it.
    Injected,
    /// Ahead of the `session/load` response.
    BeforeLoadResponse,
    /// After the `session/load` response, while Ora is still preparing the prompt.
    WhileApplyingModel,
}

/// Builds a catalog update holding exactly one command.
fn catalog(name: &str) -> SessionUpdate {
    SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(vec![
        AvailableCommand::new(name, "Runs it"),
    ]))
}

/// Frames one session update as the plugin forwards it.
fn update_frame(session_id: &str, update: SessionUpdate) -> Value {
    json!({"jsonrpc":"2.0", "method":"session/update",
        "params": SessionNotification::new(session_id.to_string(), update)})
}

/// Writes one ACP frame through the plugin envelope; a closed pipe only means the test ended.
async fn send_frame(writer: &tokio::sync::Mutex<DuplexStream>, frame: Value) {
    let _ = write_message(
        &mut *writer.lock().await,
        &json!({"jsonrpc":"2.0", "method":"agent/acp", "params":frame}),
    )
    .await;
}

/// Answers setup immediately and prompts on request, reporting commands as scripted.
async fn catalog_agent(
    report: Report,
    mut injected: mpsc::UnboundedReceiver<Value>,
    finish: Arc<tokio::sync::Semaphore>,
    mut reader: DuplexStream,
    mut writer: DuplexStream,
    frames: mpsc::UnboundedSender<Value>,
    exited: watch::Sender<bool>,
) {
    let registration = json!({"jsonrpc":"2.0", "method":"ora/register",
        "params":{"methods":[], "emits":["agent/acp"]}});
    write_message(&mut writer, &registration).await.unwrap();
    let writer = Arc::new(tokio::sync::Mutex::new(writer));
    let injector = writer.clone();
    tokio::spawn(async move {
        while let Some(frame) = injected.recv().await {
            send_frame(&injector, frame).await;
        }
    });
    let model = json!([{"id":"model", "name":"Model", "category":"model", "type":"select",
        "currentValue":"a", "options":[{"value":"a","name":"A"},{"value":"b","name":"B"}]}]);
    while let Ok(Some(envelope)) = read_message(&mut reader).await {
        if envelope["method"] == "ora/shutdown" {
            break;
        }
        let frame = envelope["params"].clone();
        frames.send(frame.clone()).unwrap();
        let id = frame["id"].clone();
        let session_id = frame["params"]["sessionId"].as_str().unwrap_or_default();
        let result = match frame["method"].as_str() {
            Some("session/new") => json!({"sessionId":"fresh"}),
            Some("session/load") => {
                if let Report::BeforeLoadResponse = report {
                    send_frame(&writer, update_frame(session_id, catalog("restored"))).await;
                }
                json!({"configOptions":model})
            }
            Some("session/set_config_option") => {
                if let Report::WhileApplyingModel = report {
                    send_frame(&writer, update_frame(session_id, catalog("restored"))).await;
                }
                json!({"configOptions":model})
            }
            Some("session/prompt") => {
                let (writer, finish) = (writer.clone(), finish.clone());
                tokio::spawn(async move {
                    finish.acquire().await.unwrap().forget();
                    let result = PromptResponse::new(StopReason::EndTurn);
                    send_frame(&writer, json!({"jsonrpc":"2.0", "id":id, "result":result})).await;
                });
                continue;
            }
            _ => continue,
        };
        send_frame(&writer, json!({"jsonrpc":"2.0", "id":id, "result":result})).await;
    }
    exited.send_replace(true);
}

/// The fixture plus the test's control over what the Agent says and when turns end.
struct CatalogFixture {
    runtime: ReviewDrainFixture,
    inject: mpsc::UnboundedSender<Value>,
    finish: Arc<tokio::sync::Semaphore>,
}

impl CatalogFixture {
    /// Composes the production router and actors over an Agent reporting as scripted.
    async fn new(report: Report) -> Self {
        let (inject, injected) = mpsc::unbounded_channel();
        let finish = Arc::new(tokio::sync::Semaphore::new(0));
        let agent_finish = finish.clone();
        let runtime = ReviewDrainFixture::new_with(move |reader, writer, _, frames, exited| {
            catalog_agent(
                report,
                injected,
                agent_finish,
                reader,
                writer,
                frames,
                exited,
            )
        })
        .await;
        Self {
            runtime,
            inject,
            finish,
        }
    }

    /// Delivers one update while no prompt runs and lets the actor settle it.
    async fn report_idle(&self, session_id: &str, update: SessionUpdate) {
        self.inject.send(update_frame(session_id, update)).unwrap();
        // The paused clock only advances once every task, the actor included, has gone quiet.
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    /// Runs one turn to completion and returns the session updates its stream carried.
    async fn turn(&self, request: PromptSessionRequest) -> Vec<SessionUpdate> {
        self.finish.add_permits(1);
        let mut stream = self.runtime.manager.prompt_session(request).await.unwrap();
        let mut updates = Vec::new();
        while let Some(event) = stream.recv().await {
            match event.unwrap() {
                PromptSessionEvent::SessionUpdate { update, .. } => updates.push(update),
                PromptSessionEvent::Completed { .. } => break,
                unexpected => panic!("unexpected prompt event: {unexpected:?}"),
            }
        }
        updates
    }

    /// Opens a load of one session.
    async fn load(&self, session_id: &str) -> SessionEventStream<LoadSessionEvent> {
        self.runtime
            .manager
            .load_session(LoadSessionRequest {
                session_id: session_id.into(),
            })
            .await
            .unwrap()
    }
}

/// Keeps only the catalogs among a turn's updates.
fn catalogs(updates: Vec<SessionUpdate>) -> Vec<SessionUpdate> {
    updates
        .into_iter()
        .filter(|update| matches!(update, SessionUpdate::AvailableCommandsUpdate(_)))
        .collect()
}

/// Reads the first event of a load, which is where session state precedes the transcript.
async fn first_load_update(
    stream: &mut SessionEventStream<LoadSessionEvent>,
) -> Option<SessionUpdate> {
    while let Some(event) = stream.recv().await {
        match event.unwrap() {
            LoadSessionEvent::SessionUpdate { update, .. } => match update {
                SessionUpdate::ConfigOptionUpdate(_) => continue,
                update => return Some(update),
            },
            _ => return None,
        }
    }
    None
}

/// A catalog reported after the setup window closed still reaches the first prompt, once.
#[test]
fn a_catalog_reported_after_session_new_reaches_the_first_prompt() {
    run_case(async {
        let fixture = CatalogFixture::new(Report::Injected).await;
        let started = fixture
            .runtime
            .manager
            .start_session(StartSessionRequest {
                workspace_id: "workspace".into(),
                agent_ref: serde_json::from_value(json!("official/test.agent")).unwrap(),
                model: None,
            })
            .await
            .unwrap();
        assert_eq!(started.available_commands, Vec::new());
        fixture.report_idle("fresh", catalog("late")).await;

        let id = started.session.id;
        assert_eq!(fixture.turn(prompt(&id)).await, vec![catalog("late")]);
        assert_eq!(fixture.turn(prompt(&id)).await, Vec::new());
        fixture.runtime.shutdown().await;
    });
}

/// Between turns only the newest catalog survives, and turn traffic still never crosses turns.
#[test]
fn an_idle_session_keeps_only_its_newest_catalog() {
    run_case(async {
        let fixture = CatalogFixture::new(Report::Injected).await;
        fixture.runtime.add_session("restored");
        fixture.turn(prompt("restored")).await;

        fixture.report_idle("restored", catalog("first")).await;
        let stray = ContentBlock::Text(TextContent::new("late output of the previous turn"));
        fixture
            .report_idle(
                "restored",
                SessionUpdate::AgentMessageChunk(ContentChunk::new(stray)),
            )
            .await;
        fixture
            .report_idle("restored", SessionUpdate::Plan(Plan::new(Vec::new())))
            .await;
        fixture.report_idle("restored", catalog("second")).await;

        assert_eq!(
            fixture.turn(prompt("restored")).await,
            vec![catalog("second")]
        );
        fixture.runtime.shutdown().await;
    });
}

/// A restore delivers the catalog whether the Agent reports it before or after answering the load.
#[test]
fn a_catalog_reported_during_restore_reaches_the_admitted_prompt() {
    for report in [Report::BeforeLoadResponse, Report::WhileApplyingModel] {
        run_case(async {
            let fixture = CatalogFixture::new(report).await;
            fixture.runtime.add_session("restored");
            let request = PromptSessionRequest {
                model: Some("b".to_string()),
                ..prompt("restored")
            };

            assert_eq!(
                catalogs(fixture.turn(request).await),
                vec![catalog("restored")]
            );
            fixture.runtime.shutdown().await;
        });
    }
}

/// A reloaded window recovers the catalog from the attached session, idle or mid-turn.
#[test]
fn a_load_replays_the_catalog_of_an_attached_session() {
    run_case(async {
        let fixture = CatalogFixture::new(Report::BeforeLoadResponse).await;
        fixture.runtime.add_session("restored");
        let mut detached = fixture.load("restored").await;
        assert_eq!(first_load_update(&mut detached).await, None);
        drop(detached);
        fixture.turn(prompt("restored")).await;

        let mut idle = fixture.load("restored").await;
        assert_eq!(
            first_load_update(&mut idle).await,
            Some(catalog("restored"))
        );
        drop(idle);

        let mut owner = fixture
            .runtime
            .manager
            .prompt_session(prompt("restored"))
            .await
            .unwrap();
        // The turn itself replaces the catalog, so the follower must see the replacement.
        fixture
            .inject
            .send(update_frame("restored", catalog("replaced")))
            .unwrap();
        assert!(matches!(
            owner.recv().await.unwrap().unwrap(),
            PromptSessionEvent::SessionUpdate { update, .. } if update == catalog("replaced")
        ));
        let mut follower = fixture.load("restored").await;
        assert_eq!(
            first_load_update(&mut follower).await,
            Some(catalog("replaced"))
        );
        fixture.finish.add_permits(1);
        assert_eq!(completion(&mut owner).await, StopReason::EndTurn);
        drop((owner, follower));
        fixture.runtime.shutdown().await;
    });
}

/// A catalog describes one attachment, so it is not replayed once the Agent process is replaced.
#[test]
fn a_detached_session_replays_no_catalog() {
    run_case(async {
        let fixture = CatalogFixture::new(Report::BeforeLoadResponse).await;
        fixture.runtime.add_session("restored");
        fixture.turn(prompt("restored")).await;
        fixture
            .runtime
            .manager
            .stop_session(ora_contracts::StopSessionRequest {
                session_id: "restored".into(),
            })
            .await
            .unwrap();

        // The transcript's first user message is all a detached load has to say first.
        let mut detached = fixture.load("restored").await;
        assert!(matches!(
            first_load_update(&mut detached).await,
            Some(SessionUpdate::UserMessageChunk(_))
        ));
        drop(detached);
        fixture.runtime.shutdown().await;
    });
}
