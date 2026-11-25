//! End-to-end runtime tests for Skill catalog delivery, deltas, resets, and ACP setup fields.
//!
//! Exercises the real `AgentRuntimeManager`, `RuntimeActor`, `SessionRecorder`, `AcpPeer`, and
//! `PluginRuntime` protocol boundary to satisfy obligations in:
//! - `specs/test-cases/desktop/core/agent/session/skill/catalog-delivery.md`
//! - `specs/test-cases/desktop/core/agent/session/skill/session-setup-and-hosts.md`

use super::{PipeProcess, PipeSpawner};
use crate::host::{
    AgentAttach, AgentPluginAttachment, AgentRuntimeHost, SessionSetup, SkillCatalogEntry,
    SkillCatalogSource, WorkspaceDirectory,
};
use crate::session_setup::{AgentSessionMcpCapabilities, SessionMcpRevision, SessionMcpSnapshot};
use crate::test_host::{NeverFails, RecordedEvents};
use crate::{
    AgentRuntimeManager, AgentRuntimeSetup, MemorySessionStore, RuntimeError,
    SKILL_DIRECTORIES_META_KEY,
};
use agent_client_protocol_schema::v1::{
    ContentBlock, MessageId, SessionUpdate, StopReason, TextContent,
};
use ora_contracts::{
    LoadSessionEvent, LoadSessionRequest, PromptSessionEvent, PromptSessionRequest,
    StartSessionRequest, StopSessionRequest,
};
use ora_domain::{
    AgentRef, PluginId, PromptInactivityPolicy, SessionHostCapabilities, SessionMcpSelection,
    WorkspaceId,
};
use ora_plugin_lifecycle::{ConnectionError, InboundNotification, PluginGenerationKey};
use ora_plugin_protocol::{read_message, write_message};
use ora_plugin_runtime::{NoHostRequests, PluginLogSetup, PluginRuntime, PluginRuntimeConfig};
use ora_scheduler::Scheduler;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::DuplexStream;
use tokio::sync::{mpsc, watch};

const PLUGIN_ID: &str = "official/skill-test.agent";

#[derive(Clone, Default)]
struct DynamicSkillCatalog {
    entries: Arc<Mutex<Vec<SkillCatalogEntry>>>,
    roots: Arc<Mutex<Vec<PathBuf>>>,
}

impl DynamicSkillCatalog {
    fn set_entries(&self, entries: Vec<SkillCatalogEntry>) {
        *self.entries.lock().unwrap() = entries;
    }

    fn set_roots(&self, roots: Vec<PathBuf>) {
        *self.roots.lock().unwrap() = roots;
    }
}

impl SkillCatalogSource for DynamicSkillCatalog {
    fn skill_catalog(&self, _workspace_id: &WorkspaceId) -> Vec<SkillCatalogEntry> {
        self.entries.lock().unwrap().clone()
    }

    fn extra_read_roots(&self) -> Vec<PathBuf> {
        self.roots.lock().unwrap().clone()
    }
}

#[derive(Clone, Default)]
struct ControllableMcpSetup {
    selection: SessionMcpSelection,
    generation: Arc<AtomicU64>,
    fail_next_refresh: Arc<AtomicBool>,
}

impl ControllableMcpSetup {
    fn bump(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    fn fail_next(&self) {
        self.fail_next_refresh.store(true, Ordering::Release);
    }
}

impl SessionSetup for ControllableMcpSetup {
    fn with_selection(&self, selection: SessionMcpSelection) -> Self {
        Self {
            selection,
            generation: Arc::clone(&self.generation),
            fail_next_refresh: Arc::clone(&self.fail_next_refresh),
        }
    }

    fn selection(&self) -> &SessionMcpSelection {
        &self.selection
    }

    fn desired_mcp_revision(&self) -> Result<SessionMcpRevision, RuntimeError> {
        Ok(SessionMcpRevision::for_generation(
            self.generation.load(Ordering::Acquire),
            Vec::new(),
        ))
    }

    fn resolve_mcp(
        &self,
        _cwd: &Path,
        _capabilities: AgentSessionMcpCapabilities,
    ) -> Result<SessionMcpSnapshot, RuntimeError> {
        if self.fail_next_refresh.swap(false, Ordering::AcqRel) {
            return Err(RuntimeError::mcp_retry_required());
        }
        Ok(SessionMcpSnapshot::new(
            Vec::new(),
            self.desired_mcp_revision()?,
        ))
    }

    fn observe_mcp_health(&self, _session_id: &ora_domain::SessionId, _cwd: &Path) {}

    fn plugin_id_for_agent(&self, _agent_ref: &AgentRef) -> Option<PluginId> {
        PluginId::parse(PLUGIN_ID).ok()
    }
}

struct SkillTestHost;

impl AgentRuntimeHost for SkillTestHost {
    type Store = MemorySessionStore;
    type Attach = MockAgentAttach;
    type Setup = ControllableMcpSetup;
    type Skills = DynamicSkillCatalog;
    type Events = RecordedEvents;
    type Directory = FixedDir;
}

struct FixedDir(PathBuf);

impl WorkspaceDirectory for FixedDir {
    fn workspace_cwd(&self, _workspace_id: &WorkspaceId) -> Result<PathBuf, RuntimeError> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
struct MockAgentAttach {
    attachment: Mutex<Option<AgentPluginAttachment>>,
    runtime: Mutex<Option<PluginRuntime>>,
}

impl MockAgentAttach {
    #[allow(clippy::too_many_arguments)]
    async fn install(
        &self,
        root: &Path,
        provider_prefix: &'static str,
        additional_directories_supported: bool,
        legacy_effect_resources: bool,
        recorded_frames: Arc<Mutex<Vec<Value>>>,
        fail_next_prompt: Arc<AtomicBool>,
        next_prompt_updates: Arc<Mutex<Vec<Value>>>,
    ) {
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
        tokio::spawn(serve_skill_plugin(
            reader,
            writer,
            exited,
            provider_prefix,
            additional_directories_supported,
            legacy_effect_resources,
            recorded_frames,
            fail_next_prompt,
            next_prompt_updates,
        ));
        let (_level, level) = watch::channel(ora_logging::LogLevel::Info);
        let (runtime, mut notifications) = PluginRuntime::launch(
            &spawner,
            PluginRuntimeConfig {
                plugin_id: PLUGIN_ID.to_string(),
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
                directory: root.join("logs").join("gen1"),
                host_session_id: "skill-test".to_string(),
                generation: 1,
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
                        plugin_id: PluginId::parse(PLUGIN_ID).unwrap(),
                        generation: PluginGenerationKey(1),
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
    }
}

impl AgentAttach for MockAgentAttach {
    type StopError = NeverFails;

    fn installed_agent_plugins(&self) -> Vec<PluginId> {
        vec![PluginId::parse(PLUGIN_ID).unwrap()]
    }

    fn is_installed(&self, plugin_id: &PluginId) -> bool {
        plugin_id.canonical() == PLUGIN_ID
    }

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

    async fn stop_plugin(&self, _plugin_id: &PluginId) -> Result<(), NeverFails> {
        let runtime = self.runtime.lock().unwrap().take();
        if let Some(runtime) = runtime {
            runtime.shutdown_and_wait().await;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn serve_skill_plugin(
    mut reader: DuplexStream,
    mut writer: DuplexStream,
    exited: watch::Sender<bool>,
    provider_prefix: &'static str,
    additional_directories_supported: bool,
    legacy_effect_resources: bool,
    recorded_frames: Arc<Mutex<Vec<Value>>>,
    fail_next_prompt: Arc<AtomicBool>,
    next_prompt_updates: Arc<Mutex<Vec<Value>>>,
) {
    let mut register = json!({
        "jsonrpc": "2.0",
        "method": "ora/register",
        "params": {
            "methods": ["agent/start", "agent/stop", "agent/list_models"],
            "emits": ["agent/acp"]
        }
    });
    if legacy_effect_resources {
        register["params"]["methods"] = json!([
            "agent/start",
            "agent/stop",
            "agent/list_models",
            "effect/coordinate",
            "effect/reactivate",
            "effect/verify_ready"
        ]);
        register["params"]["effectResources"] = json!([{
            "workspaceRelativePath": ".claude/skills",
            "materializationFormat": "ora/skill-directory.v1",
            "coordination": "quiesce_before_mutation"
        }]);
    }
    write_message(&mut writer, &register).await.unwrap();

    let mut sessions = 0_u32;
    while let Ok(Some(envelope)) = read_message(&mut reader).await {
        let method = envelope["method"].as_str().unwrap_or("");
        if method.starts_with("effect/") {
            panic!("host must never invoke effect/* methods: {envelope:?}");
        }
        let response = match method {
            "ora/shutdown" => break,
            "agent/start" => json!({
                "jsonrpc": "2.0",
                "id": envelope["id"],
                "result": {"protocol": "acp", "acpVersion": 1}
            }),
            "agent/stop" => json!({"jsonrpc": "2.0", "id": envelope["id"], "result": {}}),
            "agent/acp" => {
                let frame = &envelope["params"];
                recorded_frames.lock().unwrap().push(frame.clone());
                let acp_method = frame["method"].as_str().unwrap_or("");
                match acp_method {
                    "initialize" => {
                        let session_caps = if additional_directories_supported {
                            json!({"additionalDirectories": {}})
                        } else {
                            json!({})
                        };
                        json!({
                            "jsonrpc": "2.0",
                            "method": "agent/acp",
                            "params": {
                                "jsonrpc": "2.0",
                                "id": frame["id"],
                                "result": {
                                    "protocolVersion": 1,
                                    "agentCapabilities": {
                                        "loadSession": true,
                                        "sessionCapabilities": session_caps
                                    }
                                }
                            }
                        })
                    }
                    "session/new" => {
                        sessions += 1;
                        json!({
                            "jsonrpc": "2.0",
                            "method": "agent/acp",
                            "params": {
                                "jsonrpc": "2.0",
                                "id": frame["id"],
                                "result": {"sessionId": format!("provider-{provider_prefix}-{sessions}")}
                            }
                        })
                    }
                    "session/load" => json!({
                        "jsonrpc": "2.0",
                        "method": "agent/acp",
                        "params": {
                            "jsonrpc": "2.0",
                            "id": frame["id"],
                            "result": {}
                        }
                    }),
                    "session/prompt" => {
                        if fail_next_prompt.swap(false, Ordering::AcqRel) {
                            json!({
                                "jsonrpc": "2.0",
                                "method": "agent/acp",
                                "params": {
                                    "jsonrpc": "2.0",
                                    "id": frame["id"],
                                    "error": {"code": -32603, "message": "synthetic prompt failure"}
                                }
                            })
                        } else {
                            let sid = frame["params"]["sessionId"].as_str().unwrap();
                            let extra_updates: Vec<Value> = {
                                let mut queued = next_prompt_updates.lock().unwrap();
                                std::mem::take(&mut *queued)
                            };
                            for update in extra_updates {
                                write_message(
                                    &mut writer,
                                    &json!({
                                        "jsonrpc": "2.0",
                                        "method": "agent/acp",
                                        "params": {
                                            "jsonrpc": "2.0",
                                            "method": "session/update",
                                            "params": {
                                                "sessionId": sid,
                                                "update": update
                                            }
                                        }
                                    }),
                                )
                                .await
                                .unwrap();
                            }
                            write_message(
                                &mut writer,
                                &json!({
                                    "jsonrpc": "2.0",
                                    "method": "agent/acp",
                                    "params": {
                                        "jsonrpc": "2.0",
                                        "method": "session/update",
                                        "params": {
                                            "sessionId": sid,
                                            "update": {
                                                "sessionUpdate": "agent_message_chunk",
                                                "content": {"type": "text", "text": "ok"}
                                            }
                                        }
                                    }
                                }),
                            )
                            .await
                            .unwrap();
                            json!({
                                "jsonrpc": "2.0",
                                "method": "agent/acp",
                                "params": {
                                    "jsonrpc": "2.0",
                                    "id": frame["id"],
                                    "result": {"stopReason": "end_turn"}
                                }
                            })
                        }
                    }
                    other => panic!("unexpected ACP method: {other}"),
                }
            }
            other => panic!("unexpected plugin method: {other}"),
        };
        if write_message(&mut writer, &response).await.is_err() {
            break;
        }
    }
    exited.send_replace(true);
}

fn entry(name: &str, desc: &str, path: &str, dir: &str) -> SkillCatalogEntry {
    SkillCatalogEntry {
        name: name.to_string(),
        description: desc.to_string(),
        skill_md_path: PathBuf::from(path),
        package_dir: PathBuf::from(dir),
    }
}

async fn send_prompt(manager: &AgentRuntimeManager<SkillTestHost>, session_id: &str, text: &str) {
    let mut stream = manager
        .prompt_session(PromptSessionRequest {
            session_id: session_id.to_string(),
            prompt: vec![ContentBlock::Text(TextContent::new(text))],
            record_prompt: None,
            model: None,
        })
        .await
        .unwrap();
    let mut completed = false;
    while let Some(event) = stream.recv().await {
        if let PromptSessionEvent::Completed { stop_reason, .. } = event.unwrap() {
            assert_eq!(stop_reason, StopReason::EndTurn);
            completed = true;
        }
    }
    assert!(completed);
}

fn acp_methods<'a>(frames: &'a [Value], method: &str) -> Vec<&'a Value> {
    frames
        .iter()
        .filter(|frame| frame["method"] == method)
        .collect()
}

fn prompt_texts(prompt_frame: &Value) -> Vec<String> {
    prompt_frame["params"]["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| block["text"].as_str().unwrap().to_string())
        .collect()
}

/// Verifies send-time catalog injection, deltas, admission failure preservation, MCP refresh
/// retention, cold-start/stop full reset, slash command ordering, capability negotiation, legacy
/// `effectResources` tolerance, and clean user history recording across real actors and ACP wire.
///
/// Covers `catalog-delivery.md` and `session-setup-and-hosts.md`.
async fn run_skill_catalog_e2e_scenario() {
    ora_logging::initialize_test_clock();
    let dir = TempDir::new().unwrap();
    let frames = Arc::new(Mutex::new(Vec::new()));
    let fail_next_prompt = Arc::new(AtomicBool::new(false));
    let next_prompt_updates = Arc::new(Mutex::new(Vec::new()));
    let attach = Arc::new(MockAgentAttach::default());
    attach
        .install(
            dir.path(),
            /*provider_prefix*/ "a",
            /*additional_directories_supported*/ true,
            /*legacy_effect_resources*/ true,
            frames.clone(),
            fail_next_prompt.clone(),
            next_prompt_updates.clone(),
        )
        .await;

    let skills = DynamicSkillCatalog::default();
    let read_roots = vec![
        dir.path().join("atoms").join("skills"),
        dir.path().join("plugins").join("installed"),
    ];
    skills.set_roots(read_roots.clone());

    let mcp = ControllableMcpSetup::default();
    let store = MemorySessionStore::default();
    let scheduler = Scheduler::new(chrono_tz::Asia::Shanghai);
    let manager = AgentRuntimeManager::new(AgentRuntimeSetup::<SkillTestHost> {
        attach: attach.clone(),
        store: store.clone(),
        directory: FixedDir(dir.path().to_path_buf()),
        session_setup: mcp.clone(),
        skill_catalog: skills.clone(),
        events: RecordedEvents::default(),
        home_directory: dir.path().to_path_buf(),
        sessions_root: dir.path().join("sessions"),
        scheduler: scheduler.clone(),
    })
    .unwrap();
    manager
        .wait_for_agent(&AgentRef::parse(PLUGIN_ID).unwrap())
        .await
        .unwrap();

    // Start session while catalog is empty; then add a skill BEFORE the first prompt so we verify
    // send-time catalog reading (catalog-delivery.md#first-prompt-must-carry-the-full-catalog-of-available-skills).
    let session = manager
        .start_session(StartSessionRequest {
            workspace_id: "ws-1".into(),
            agent_ref: PLUGIN_ID.into(),
            model: None,
        })
        .await
        .unwrap()
        .session;
    {
        let recorded = frames.lock().unwrap();
        let news = acp_methods(&recorded, "session/new");
        assert_eq!(news.len(), 1);
        assert_eq!(
            news[0]["params"]["additionalDirectories"],
            serde_json::to_value(&read_roots).unwrap()
        );
        assert!(news[0]["params"].get("_meta").is_none());
    }

    let skill_a = entry(
        "alpha",
        "Alpha skill.",
        &dir.path()
            .join("atoms/skills/alpha/SKILL.md")
            .to_string_lossy(),
        &dir.path().join("atoms/skills/alpha").to_string_lossy(),
    );
    skills.set_entries(vec![skill_a.clone()]);

    // Prompt 1: full catalog ahead of ordinary user prompt.
    send_prompt(&manager, &session.id, "hello alpha").await;
    {
        let recorded = frames.lock().unwrap();
        let prompts = acp_methods(&recorded, "session/prompt");
        assert_eq!(prompts.len(), 1);
        let texts = prompt_texts(prompts[0]);
        assert_eq!(texts.len(), 2);
        assert!(texts[0].starts_with("<available_skills>\n"));
        assert!(texts[0].contains("alpha: Alpha skill."));
        assert_eq!(texts[1], "hello alpha");
    }

    // Prompt 2 with identical path set: no catalog block added.
    send_prompt(&manager, &session.id, "second turn").await;
    {
        let recorded = frames.lock().unwrap();
        let prompts = acp_methods(&recorded, "session/prompt");
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompt_texts(prompts[1]), vec!["second turn".to_string()]);
    }

    // Install skill B, but make MCP refresh fail admission before sending: delta must NOT be lost.
    let skill_b = entry(
        "beta",
        "Beta skill.",
        &dir.path()
            .join("atoms/skills/beta/SKILL.md")
            .to_string_lossy(),
        &dir.path().join("atoms/skills/beta").to_string_lossy(),
    );
    skills.set_entries(vec![skill_a.clone(), skill_b.clone()]);
    mcp.fail_next();
    mcp.bump();
    let failed_admission = manager
        .prompt_session(PromptSessionRequest {
            session_id: session.id.clone(),
            prompt: vec![ContentBlock::Text(TextContent::new("blocked turn"))],
            record_prompt: None,
            model: None,
        })
        .await;
    assert!(failed_admission.is_err());

    // Prompt 3 succeeds (including MCP refresh `session/load`) with leading slash command:
    // leading `/review` stays first, `<available_skills_update>` for beta follows immediately!
    // Also MCP refresh `session/load` must carry full additionalDirectories & _meta, and MUST NOT
    // clear announced skills (so prompt 3 sends delta, not full).
    send_prompt(&manager, &session.id, "/review check diff").await;
    {
        let recorded = frames.lock().unwrap();
        let loads = acp_methods(&recorded, "session/load");
        assert_eq!(loads.len(), 1);
        assert_eq!(
            loads[0]["params"]["additionalDirectories"],
            serde_json::to_value(&read_roots).unwrap()
        );
        assert!(
            loads[0]["params"]["_meta"][SKILL_DIRECTORIES_META_KEY]
                .as_array()
                .unwrap()
                .len()
                == 2
        );

        let prompts = acp_methods(&recorded, "session/prompt");
        assert_eq!(prompts.len(), 3);
        let texts = prompt_texts(prompts[2]);
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0], "/review check diff");
        assert!(texts[1].starts_with("<available_skills_update>\n"));
        assert!(texts[1].contains("beta: Beta skill."));
        assert!(!texts[1].contains("alpha"));
    }

    // Stopping and prompting again (`session/load` restoration) must resend FULL catalog.
    manager
        .stop_session(StopSessionRequest {
            session_id: session.id.clone(),
        })
        .await
        .unwrap();
    send_prompt(&manager, &session.id, "after stop").await;
    {
        let recorded = frames.lock().unwrap();
        let loads = acp_methods(&recorded, "session/load");
        assert_eq!(loads.len(), 2);
        let prompts = acp_methods(&recorded, "session/prompt");
        assert_eq!(prompts.len(), 4);
        let texts = prompt_texts(prompts[3]);
        assert!(texts[0].starts_with("<available_skills>\n"));
        assert!(texts[0].contains("alpha"));
        assert!(texts[0].contains("beta"));
        assert_eq!(texts[1], "after stop");
    }

    // Cold restart with a brand new AgentRuntimeManager & attach over the same store & history:
    // recorded history contains ONLY user input (no `<available_skills*>`), and next prompt
    // after cold `session/load` sends full catalog again.
    drop(manager);
    let restarted_attach = Arc::new(MockAgentAttach::default());
    restarted_attach
        .install(
            dir.path(),
            /*provider_prefix*/ "b",
            /*additional_directories_supported*/ false,
            /*legacy_effect_resources*/ false,
            frames.clone(),
            fail_next_prompt.clone(),
            next_prompt_updates.clone(),
        )
        .await;
    let restarted = AgentRuntimeManager::new(AgentRuntimeSetup::<SkillTestHost> {
        attach: restarted_attach.clone(),
        store: store.clone(),
        directory: FixedDir(dir.path().to_path_buf()),
        session_setup: mcp.clone(),
        skill_catalog: skills.clone(),
        events: RecordedEvents::default(),
        home_directory: dir.path().to_path_buf(),
        sessions_root: dir.path().join("sessions"),
        scheduler: scheduler.clone(),
    })
    .unwrap();
    restarted
        .wait_for_agent(&AgentRef::parse(PLUGIN_ID).unwrap())
        .await
        .unwrap();

    // Verify user history does not contain injected catalog blocks.
    let mut load_stream = restarted
        .load_session(LoadSessionRequest {
            session_id: session.id.clone(),
        })
        .await
        .unwrap();
    let mut loaded_user_texts = Vec::new();
    while let Some(event) = load_stream.recv().await {
        if let LoadSessionEvent::SessionUpdate {
            update: SessionUpdate::UserMessageChunk(chunk),
            ..
        } = event.unwrap()
            && let ContentBlock::Text(text) = chunk.content
        {
            loaded_user_texts.push(text.text);
        }
    }
    assert_eq!(
        loaded_user_texts,
        vec![
            "hello alpha".to_string(),
            "second turn".to_string(),
            "/review check diff".to_string(),
            "after stop".to_string(),
        ]
    );

    // Agent without `additionalDirectories` capability still succeeds on `session/load` and
    // `session/new`, omits `additionalDirectories`, and delivers full `<available_skills>`.
    let frames_before_cold_prompt = frames.lock().unwrap().len();
    send_prompt(&restarted, &session.id, "cold restart prompt").await;
    {
        let recorded = frames.lock().unwrap();
        let new_frames = &recorded[frames_before_cold_prompt..];
        let loads = acp_methods(new_frames, "session/load");
        assert_eq!(loads.len(), 1);
        assert!(loads[0]["params"].get("additionalDirectories").is_none());
        let prompts = acp_methods(new_frames, "session/prompt");
        assert_eq!(prompts.len(), 1);
        assert!(prompt_texts(prompts[0])[0].starts_with("<available_skills>\n"));
    }

    // Workflow node session uses the same catalog delivery logic.
    let wf = restarted
        .start_workflow_node_session(
            StartSessionRequest {
                workspace_id: "ws-1".into(),
                agent_ref: PLUGIN_ID.into(),
                model: None,
            },
            SessionMcpSelection::Automatic,
            SessionHostCapabilities::None,
        )
        .await
        .unwrap()
        .session;
    let mut wf_stream = restarted
        .prompt_session_with_inactivity_policy(
            PromptSessionRequest {
                session_id: wf.id,
                prompt: vec![ContentBlock::Text(TextContent::new(
                    "/alpha\n\n<required_skills>...</required_skills>",
                ))],
                record_prompt: None,
                model: None,
            },
            PromptInactivityPolicy::Wait,
        )
        .await
        .unwrap();
    while let Some(event) = wf_stream.recv().await {
        if let PromptSessionEvent::Completed { .. } = event.unwrap() {
            break;
        }
    }
    {
        let recorded = frames.lock().unwrap();
        let prompts = acp_methods(&recorded, "session/prompt");
        let wf_texts = prompt_texts(prompts.last().unwrap());
        assert_eq!(wf_texts.len(), 2);
        assert_eq!(
            wf_texts[0],
            "/alpha\n\n<required_skills>...</required_skills>"
        );
        assert!(wf_texts[1].starts_with("<available_skills>\n"));
    }

    // `commit_admission` failure AND wire `session/prompt` failure must keep delta pending.
    let skill_c = entry(
        "gamma",
        "Gamma skill.",
        &dir.path()
            .join("atoms/skills/gamma/SKILL.md")
            .to_string_lossy(),
        &dir.path().join("atoms/skills/gamma").to_string_lossy(),
    );
    skills.set_entries(vec![skill_a, skill_b, skill_c]);
    let frames_before_rejected = frames.lock().unwrap().len();
    let rejected = restarted
        .prompt_session_with_admission(
            PromptSessionRequest {
                session_id: session.id.clone(),
                prompt: vec![ContentBlock::Text(TextContent::new("rejected commit"))],
                record_prompt: None,
                model: None,
            },
            MessageId::new("msg-rejected"),
            || Err(RuntimeError::mcp_retry_required()),
        )
        .await;
    assert!(rejected.is_err());

    send_prompt(&restarted, &session.id, "after rejected commit").await;
    {
        let recorded = frames.lock().unwrap();
        let after_rejected = &recorded[frames_before_rejected..];
        assert!(acp_methods(after_rejected, "session/load").is_empty());
        assert!(acp_methods(after_rejected, "session/new").is_empty());
        let prompts = acp_methods(&recorded, "session/prompt");
        let texts = prompt_texts(prompts.last().unwrap());
        assert!(
            texts[0].starts_with("<available_skills_update>\n"),
            "unexpected texts: {texts:?}"
        );
        assert!(texts[0].contains("gamma: Gamma skill."));
    }

    scheduler.shutdown().await;
}

/// Verifies ADR D3 item 3 (an empty initial turn initializes the announced set as empty so a
/// skill installed later arrives as an incremental update) and ADR D4 / invariant 5 (parent
/// `opencode/compaction` completion clears the announced set so the next turn re-emits the full
/// catalog even when paths did not change, while `started`, `failed`, and child-session
/// compactions do not reset it).
async fn run_empty_init_and_compaction_reset_scenario() {
    ora_logging::initialize_test_clock();
    let dir = TempDir::new().unwrap();
    let frames = Arc::new(Mutex::new(Vec::new()));
    let fail_next_prompt = Arc::new(AtomicBool::new(false));
    let next_prompt_updates = Arc::new(Mutex::new(Vec::new()));
    let attach = Arc::new(MockAgentAttach::default());
    attach
        .install(
            dir.path(),
            /*provider_prefix*/ "compact",
            /*additional_directories_supported*/ true,
            /*legacy_effect_resources*/ false,
            frames.clone(),
            fail_next_prompt,
            next_prompt_updates.clone(),
        )
        .await;

    let skills = DynamicSkillCatalog::default();
    let mcp = ControllableMcpSetup::default();
    let store = MemorySessionStore::default();
    let scheduler = Scheduler::new(chrono_tz::Asia::Shanghai);
    let manager = AgentRuntimeManager::new(AgentRuntimeSetup::<SkillTestHost> {
        attach,
        store,
        directory: FixedDir(dir.path().to_path_buf()),
        session_setup: mcp,
        skill_catalog: skills.clone(),
        events: RecordedEvents::default(),
        home_directory: dir.path().to_path_buf(),
        sessions_root: dir.path().join("sessions"),
        scheduler: scheduler.clone(),
    })
    .unwrap();
    manager
        .wait_for_agent(&AgentRef::parse(PLUGIN_ID).unwrap())
        .await
        .unwrap();

    let session = manager
        .start_session(StartSessionRequest {
            workspace_id: "ws-1".into(),
            agent_ref: PLUGIN_ID.into(),
            model: None,
        })
        .await
        .unwrap()
        .session;

    // Turn 1 with empty catalog: no block injected, announced set becomes empty (not uninitialized).
    send_prompt(&manager, &session.id, "first empty turn").await;
    {
        let recorded = frames.lock().unwrap();
        let prompts = acp_methods(&recorded, "session/prompt");
        assert_eq!(prompt_texts(prompts[0]), vec!["first empty turn"]);
    }

    // Install first skill; Turn 2 must emit incremental `<available_skills_update>`, not full.
    let skill_a = entry(
        "alpha",
        "Alpha skill.",
        &dir.path()
            .join("atoms/skills/alpha/SKILL.md")
            .to_string_lossy(),
        &dir.path().join("atoms/skills/alpha").to_string_lossy(),
    );
    skills.set_entries(vec![skill_a]);
    // Also deliver non-resetting compaction notices during Turn 2: parent started, parent failed,
    // and child-session completed must NOT clear the parent session's announced set.
    *next_prompt_updates.lock().unwrap() = vec![
        json!({
            "sessionUpdate": "session_info_update",
            "_meta": { "opencode/compaction": { "status": "started" } }
        }),
        json!({
            "sessionUpdate": "session_info_update",
            "_meta": { "opencode/compaction": { "status": "failed" } }
        }),
        json!({
            "sessionUpdate": "session_info_update",
            "_meta": {
                "opencode/child-session": { "id": "child-1", "parentID": "parent-1", "depth": 1 },
                "opencode/compaction": { "status": "completed" }
            }
        }),
    ];
    send_prompt(&manager, &session.id, "install first skill").await;
    {
        let recorded = frames.lock().unwrap();
        let prompts = acp_methods(&recorded, "session/prompt");
        let texts = prompt_texts(prompts[1]);
        assert_eq!(texts.len(), 2);
        assert!(
            texts[0].starts_with("<available_skills_update>\n"),
            "expected incremental update after empty turn 1, got: {texts:?}"
        );
        assert!(texts[0].contains("alpha: Alpha skill."));
        assert_eq!(texts[1], "install first skill");
    }

    // Turn 3: path set unchanged and no parent compaction completed -> no block injected.
    // During Turn 3, provider reports parent `opencode/compaction` completed.
    *next_prompt_updates.lock().unwrap() = vec![json!({
        "sessionUpdate": "session_info_update",
        "_meta": { "opencode/compaction": { "status": "completed" } }
    })];
    send_prompt(&manager, &session.id, "trigger compaction").await;
    {
        let recorded = frames.lock().unwrap();
        let prompts = acp_methods(&recorded, "session/prompt");
        assert_eq!(prompt_texts(prompts[2]), vec!["trigger compaction"]);
    }

    // Turn 4: path set still unchanged, but compaction cleared announced skills -> full catalog!
    send_prompt(&manager, &session.id, "after compaction").await;
    {
        let recorded = frames.lock().unwrap();
        let prompts = acp_methods(&recorded, "session/prompt");
        let texts = prompt_texts(prompts[3]);
        assert_eq!(texts.len(), 2);
        assert!(
            texts[0].starts_with("<available_skills>\n"),
            "expected full catalog after parent compaction completion, got: {texts:?}"
        );
        assert!(texts[0].contains("alpha: Alpha skill."));
        assert_eq!(texts[1], "after compaction");
    }

    scheduler.shutdown().await;
}

#[tokio::test]
async fn skill_catalog_delivery_deltas_resets_capabilities_and_history_across_live_sessions() {
    tokio::time::timeout(Duration::from_secs(10), run_skill_catalog_e2e_scenario())
        .await
        .expect("skill catalog e2e scenario timed out");
}

#[tokio::test]
async fn empty_first_turn_emits_delta_on_first_install_and_compaction_resets_to_full_catalog() {
    tokio::time::timeout(
        Duration::from_secs(10),
        run_empty_init_and_compaction_reset_scenario(),
    )
    .await
    .expect("empty init and compaction reset scenario timed out");
}
