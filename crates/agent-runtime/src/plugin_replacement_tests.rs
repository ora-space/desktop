//! Replaces a plugin at the host boundary while keeping the real supervisor and session actor.

use super::{PipeProcess, PipeSpawner, prompt};
use crate::host::{AgentAttach, AgentPluginAttachment, AgentRuntimeHost, WorkspaceDirectory};
use crate::test_host::{NeverFails, RecordedEvents};
use crate::{
    AgentRuntimeManager, AgentRuntimeSetup, MemorySessionStore, NoSessionMcp, RuntimeError,
};
use agent_client_protocol_schema::v1::{ContentBlock, SessionUpdate, StopReason};
use ora_contracts::{
    EmptyErrorParams, PromptSessionEvent, PublicError, StartSessionRequest, StopSessionRequest,
};
use ora_domain::{AgentRef, PluginId, WorkspaceId};
use ora_effect::ConsumerDeclaration;
use ora_plugin_lifecycle::{ConnectionError, InboundNotification, PluginGenerationKey};
use ora_plugin_protocol::{read_message, write_message};
use ora_plugin_runtime::{NoHostRequests, PluginLogSetup, PluginRuntime, PluginRuntimeConfig};
use ora_scheduler::Scheduler;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::DuplexStream;
use tokio::sync::{mpsc, watch};

const PLUGIN: &str = "official/replacement.agent";

struct ReplacementHost;

impl AgentRuntimeHost for ReplacementHost {
    type Store = MemorySessionStore;
    type Attach = InstalledPlugin;
    type Setup = NoSessionMcp;
    type Events = RecordedEvents;
    type Directory = Directory;
}

struct Directory(PathBuf);

impl WorkspaceDirectory for Directory {
    /// Resolves the test Workspace without changing any user directory.
    fn workspace_cwd(&self, _workspace_id: &WorkspaceId) -> Result<PathBuf, RuntimeError> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
struct InstalledPlugin {
    installed: Mutex<bool>,
    attachment: Mutex<Option<AgentPluginAttachment>>,
    runtime: Mutex<Option<PluginRuntime>>,
}

impl InstalledPlugin {
    /// Publishes a new process incarnation through the same host interface used by Desktop.
    async fn install(&self, root: &Path, generation: u64, load_supported: bool) {
        let entrypoint = root.join("agent.ts");
        std::fs::write(&entrypoint, "").unwrap();
        let (stdin, reader) = tokio::io::duplex(/*max_buf_size*/ 65536);
        let (writer, stdout) = tokio::io::duplex(/*max_buf_size*/ 65536);
        let (exited, _) = watch::channel(false);
        let spawner = PipeSpawner(Mutex::new(Some(PipeProcess {
            stdin: Some(stdin),
            stdout: Some(stdout),
            exited: exited.clone(),
        })));
        tokio::spawn(serve_plugin(
            reader,
            writer,
            exited,
            generation,
            load_supported,
        ));
        let (_level, level) = watch::channel(ora_logging::LogLevel::Info);
        let (runtime, mut notifications) = PluginRuntime::launch(
            &spawner,
            PluginRuntimeConfig {
                plugin_id: PLUGIN.to_string(),
                deno_path: root.join("unused-deno"),
                entrypoint,
                permissions: Vec::new(),
                cwd: None,
                environment: BTreeMap::new(),
                ready_timeout: Duration::from_secs(5),
                call_timeout: Duration::from_secs(5),
                shutdown_timeout: Duration::from_secs(1),
            },
            NoHostRequests,
            PluginLogSetup {
                root: root.join("logs"),
                directory: root.join("logs").join(generation.to_string()),
                host_session_id: "replacement-test".to_string(),
                generation,
                level,
            },
        )
        .await
        .unwrap();
        let (sender, receiver) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(notification) = notifications.recv().await {
                if sender
                    .send(InboundNotification {
                        plugin_id: PluginId::parse(PLUGIN).unwrap(),
                        generation: PluginGenerationKey(generation),
                        method: notification.method,
                        params: notification.params,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        *self.runtime.lock().unwrap() = Some(runtime.clone());
        *self.attachment.lock().unwrap() = Some(AgentPluginAttachment {
            runtime,
            notifications: receiver,
        });
        *self.installed.lock().unwrap() = true;
    }
}

impl AgentAttach for InstalledPlugin {
    type StopError = NeverFails;

    /// Reports whether this generation's package is still installed.
    fn installed_agent_plugins(&self) -> Vec<PluginId> {
        if *self.installed.lock().unwrap() {
            vec![PluginId::parse(PLUGIN).unwrap()]
        } else {
            Vec::new()
        }
    }

    /// Keeps package identity stable across replacement generations.
    fn is_installed(&self, plugin_id: &PluginId) -> bool {
        plugin_id.canonical() == PLUGIN && *self.installed.lock().unwrap()
    }

    /// Transfers the process's lossless notification tap to the production supervisor.
    async fn attach_agent(
        &self,
        _plugin_id: &PluginId,
    ) -> Result<AgentPluginAttachment, ConnectionError> {
        self.attachment
            .lock()
            .unwrap()
            .take()
            .ok_or(ConnectionError::NoProcess)
    }

    /// Waits for the retired plugin to exit before another incarnation can be installed.
    async fn stop_plugin(&self, _plugin_id: &PluginId) -> Result<(), NeverFails> {
        let runtime = self.runtime.lock().unwrap().take();
        if let Some(runtime) = runtime {
            runtime.shutdown_and_wait().await;
        }
        Ok(())
    }

    /// This test agent has no Effect resources.
    fn replace_agent_effect_declaration(
        &self,
        _plugin_id: PluginId,
        _declaration: Option<ConsumerDeclaration>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
}

/// Speaks real plugin envelopes and ACP, with both restoring and rebuilding providers.
async fn serve_plugin(
    mut reader: DuplexStream,
    mut writer: DuplexStream,
    exited: watch::Sender<bool>,
    generation: u64,
    load_supported: bool,
) {
    write_message(
        &mut writer,
        &json!({"jsonrpc":"2.0","method":"ora/register","params":{
            "methods":["agent/start","agent/stop","agent/list_models"],"emits":["agent/acp"]
        }}),
    )
    .await
    .unwrap();
    let mut sessions = 0;
    while let Ok(Some(envelope)) = read_message(&mut reader).await {
        let response = match envelope["method"].as_str() {
            Some("ora/shutdown") => break,
            Some("agent/start") => {
                json!({"jsonrpc":"2.0","id":envelope["id"],"result":{"protocol":"acp","acpVersion":1}})
            }
            Some("agent/stop") => json!({"jsonrpc":"2.0","id":envelope["id"],"result":{}}),
            Some("agent/acp") => {
                let frame = &envelope["params"];
                let result = match frame["method"].as_str() {
                    Some("initialize") => {
                        json!({"protocolVersion":1,"agentCapabilities":{"loadSession":load_supported}})
                    }
                    Some("session/new") => {
                        sessions += 1;
                        json!({"sessionId":format!("provider-{generation}-{sessions}")})
                    }
                    Some("session/load") => json!({}),
                    Some("session/prompt") => {
                        write_message(&mut writer, &json!({"jsonrpc":"2.0","method":"agent/acp","params":{
                            "jsonrpc":"2.0","method":"session/update","params":{
                                "sessionId":frame["params"]["sessionId"],"update":{
                                    "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":format!("reply from generation {generation}")}
                                }
                            }
                        }})).await.unwrap();
                        json!({"stopReason":"end_turn"})
                    }
                    other => panic!("unexpected ACP request: {other:?}"),
                };
                json!({"jsonrpc":"2.0","method":"agent/acp","params":{"jsonrpc":"2.0","id":frame["id"],"result":result}})
            }
            other => panic!("unexpected plugin request: {other:?}"),
        };
        if write_message(&mut writer, &response).await.is_err() {
            break;
        }
    }
    exited.send_replace(true);
}

/// Proves the admitted turn actually reached the expected process incarnation.
async fn assert_reply(manager: &AgentRuntimeManager<ReplacementHost>, id: &str, generation: u64) {
    let mut stream = manager
        .prompt_session(prompt(id))
        .await
        .expect("existing session must use the installed agent");
    let mut replies = Vec::new();
    let mut completion = None;
    while let Some(event) = stream.recv().await {
        match event.unwrap() {
            PromptSessionEvent::SessionUpdate {
                update: SessionUpdate::AgentMessageChunk(chunk),
                ..
            } => {
                if let ContentBlock::Text(text) = chunk.content {
                    replies.push(text.text);
                }
            }
            PromptSessionEvent::Completed { stop_reason, .. } => completion = Some(stop_reason),
            _ => {}
        }
    }
    assert_eq!(
        (replies, completion),
        (
            vec![format!("reply from generation {generation}")],
            Some(StopReason::EndTurn)
        )
    );
}

/// Reinstall has an observable absent interval; update keeps the package identity installed.
enum Replacement {
    Reinstall,
    Update,
}

/// Keeps the same actor through removal and two installations, then compares old and new sessions.
async fn replacement_case(load_supported: bool) {
    let directory = TempDir::new().unwrap();
    let host = Arc::new(InstalledPlugin::default());
    host.install(directory.path(), /*generation*/ 1, load_supported)
        .await;
    let scheduler = Scheduler::new(chrono_tz::Asia::Shanghai);
    let manager = AgentRuntimeManager::new(AgentRuntimeSetup::<ReplacementHost> {
        attach: host.clone(),
        store: MemorySessionStore::default(),
        directory: Directory(directory.path().to_path_buf()),
        session_setup: NoSessionMcp::default(),
        events: RecordedEvents::default(),
        home_directory: directory.path().to_path_buf(),
        sessions_root: directory.path().join("sessions"),
        scheduler: scheduler.clone(),
    })
    .unwrap();
    let agent = AgentRef::parse(PLUGIN).unwrap();
    manager.wait_for_agent(&agent).await.unwrap();
    let request = || StartSessionRequest {
        workspace_id: "workspace".to_string(),
        agent_ref: PLUGIN.to_string(),
        model: None,
    };
    let existing = manager.start_session(request()).await.unwrap().session.id;
    assert_reply(&manager, &existing, /*generation*/ 1).await;
    for (generation, replacement) in [(2, Replacement::Reinstall), (3, Replacement::Update)] {
        let old = manager.inner.connections.for_agent(&agent).unwrap();
        let mut retired = old.state.clone();
        manager.suspend_plugin_agent(PLUGIN);
        if matches!(replacement, Replacement::Reinstall) {
            *host.installed.lock().unwrap() = false;
        }
        host.stop_plugin(&PluginId::parse(PLUGIN).unwrap())
            .await
            .unwrap();
        while retired.changed().await.is_ok() {}
        if matches!(replacement, Replacement::Reinstall) {
            manager.resume_plugin_agent(PLUGIN);
            manager.sync_plugin_agents();
            let unavailable = match manager.prompt_session(prompt(&existing)).await {
                Ok(_) => panic!("an uninstalled agent accepted a prompt"),
                Err(error) => error,
            };
            assert_eq!(
                unavailable.public_error(),
                &PublicError::AgentRuntimeUnavailable(EmptyErrorParams {})
            );
        }
        host.install(directory.path(), generation, load_supported)
            .await;
        if matches!(replacement, Replacement::Update) {
            manager.resume_plugin_agent(PLUGIN);
        }
        manager.sync_plugin_agents();
        manager.wait_for_agent(&agent).await.unwrap();
        // Reaching Ready proves installation succeeded; this exact admission used to keep
        // consulting the retired supervisor even though newly-created sessions could work.
        for _ in 0..2 {
            assert_reply(&manager, &existing, generation).await;
        }
    }
    let fresh = manager.start_session(request()).await.unwrap().session.id;
    assert_reply(&manager, &fresh, /*generation*/ 3).await;
    for session_id in [existing, fresh] {
        manager
            .stop_session(StopSessionRequest { session_id })
            .await
            .unwrap();
    }
    manager.suspend_plugin_agent(PLUGIN);
    host.stop_plugin(&PluginId::parse(PLUGIN).unwrap())
        .await
        .unwrap();
    scheduler.shutdown().await;
}

/// Exercises real supervisor retirement and prompt admission under a bounded test runtime.
fn run_replacement_case(load_supported: bool) {
    ora_logging::initialize_test_clock();
    ora_logging::with_trace_logging(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(15), replacement_case(load_supported))
                    .await
                    .expect("replacement must settle");
            });
    });
}

/// A restorable provider must reattach the existing conversation after every reinstall.
#[test]
fn existing_session_recovers_after_plugin_reinstall_with_load() {
    run_replacement_case(/*load_supported*/ true);
}

/// Providers without load support must rebuild and retain the same logical conversation.
#[test]
fn existing_session_recovers_after_plugin_reinstall_without_load() {
    run_replacement_case(/*load_supported*/ false);
}
