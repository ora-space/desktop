//! Exercises prompt policy through the production admission, ACP peer, router, and session actor.

#[path = "discovery_tests.rs"]
mod discovery_tests;
#[path = "plugin_replacement_tests.rs"]
mod plugin_replacement_tests;
#[path = "prompt_policy_tests/restore_replay.rs"]
mod restore_replay;
#[path = "turn_cancel_tests.rs"]
mod turn_cancel_tests;

use super::*;
use crate::host::SessionStore;
use crate::session_setup::LiveMcpState;
use crate::suspend::AgentProcess;
use crate::test_host::{TestHost, test_runtime};
use crate::{ActorSetup, MemorySessionStore, NoSessionMcp, PromptEndCause, SessionEventStream};
use agent_client_protocol_schema::v1::{
    ContentBlock, PromptResponse, SessionNotification, SessionUpdate, StopReason, TextContent,
};
use ora_acp::AcpPeer;
use ora_contracts::{
    CancelSessionPromptRequest, EmptyErrorParams, PromptSessionEvent, PromptSessionRequest,
    PublicError,
};
use ora_domain::{
    AuditFields, PromptInactivityPolicy, Session, SessionId, SessionMcpSelection, WorkspaceId,
};
use ora_plugin_protocol::{read_message, write_message};
use ora_plugin_runtime::{NoHostRequests, PluginLogSetup, PluginRuntimeConfig};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io;
use std::process::ExitStatus;
use tempfile::TempDir;
use tokio::io::DuplexStream;
use tokio::sync::oneshot;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer};

/// In-memory pipes retain the real plugin protocol without launching an OS executable.
struct PipeProcess {
    stdin: Option<DuplexStream>,
    stdout: Option<DuplexStream>,
    exited: watch::Sender<bool>,
}

impl ManagedProcess for PipeProcess {
    type Stdin = DuplexStream;
    type Stdout = DuplexStream;
    type Stderr = tokio::io::Empty;

    /// In-memory processes have no operating-system identity.
    fn id(&self) -> Option<u32> {
        None
    }
    /// Transfers the host's writer to the production plugin runtime.
    fn take_stdin(&mut self) -> Option<Self::Stdin> {
        self.stdin.take()
    }
    /// Transfers the host's reader to the production plugin runtime.
    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.stdout.take()
    }
    /// EOF stderr makes logging teardown independent of fake agent behavior.
    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        Some(tokio::io::empty())
    }
    /// Reports only the exit the fixture or runtime has actually requested.
    fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        Ok((*self.exited.borrow()).then(successful_exit))
    }
    /// Retains the process until the fake agent or shutdown confirms exit.
    async fn wait(&self) -> io::Result<ExitStatus> {
        let mut exited = self.exited.subscribe();
        while !*exited.borrow_and_update() {
            exited
                .changed()
                .await
                .map_err(|_| io::Error::other("exit watch closed"))?;
        }
        Ok(successful_exit())
    }
    /// Forceful shutdown acknowledges the same process owner used by normal exit.
    async fn kill(&self) -> io::Result<()> {
        self.exited.send_replace(true);
        Ok(())
    }
}

/// Builds the portable status produced by the in-memory process.
fn successful_exit() -> ExitStatus {
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }
}

/// Supplies exactly the pipes owned by this fixture, preventing accidental extra processes.
struct PipeSpawner(Mutex<Option<PipeProcess>>);

impl ProcessSpawner for PipeSpawner {
    type Process = PipeProcess;

    /// Admits one process owner and rejects reuse instead of manufacturing another connection.
    fn spawn(&self, _spec: ProcessSpec) -> io::Result<PipeProcess> {
        self.0
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| io::Error::other("already spawned"))
    }
}

/// Agent behavior selected at a real protocol boundary, independently of the runtime timer.
enum AgentInput {
    Finish(String, StopReason),
    Update(SessionNotification),
    RawFrame(Value),
    /// Asks the host to approve one tool call, exactly as a provider does mid-turn.
    Permission {
        session_id: String,
        tool_call: Value,
    },
    Disconnect,
    CancelReply {
        session_id: String,
        reply: CancelReply,
        configured: oneshot::Sender<()>,
    },
}

/// Distinguishes a confirmed cancellation from fences that cannot safely admit a retry.
#[derive(Clone, Copy)]
enum CancelReply {
    Confirm,
    EndTurn,
    RequestError,
    Ignore,
}

/// Holds two or more sessions on one real routed ACP peer with a controllable provider.
struct Fixture {
    _directory: TempDir,
    manager: crate::AgentRuntimeManager<TestHost>,
    supervisor: ConnectionSupervisor,
    plugin: PluginRuntime,
    agent: mpsc::UnboundedSender<AgentInput>,
    frames: mpsc::UnboundedReceiver<Value>,
    routed: tokio::task::JoinHandle<()>,
    state: watch::Sender<ConnectionState>,
    scheduler: ora_scheduler::Scheduler,
}

impl Fixture {
    /// Most policy cases begin with an attached session and need no provider restore.
    async fn new() -> Self {
        Self::with_load_support(/*load_session_supported*/ false).await
    }

    /// Composes the production peer, generation router, and actors over controlled pipes.
    async fn with_load_support(load_session_supported: bool) -> Self {
        let directory = TempDir::new().unwrap();
        let entrypoint = directory.path().join("agent.ts");
        std::fs::write(&entrypoint, "").unwrap();
        let (stdin, agent_reader) = tokio::io::duplex(/*max_buf_size*/ 65536);
        let (agent_writer, stdout) = tokio::io::duplex(/*max_buf_size*/ 65536);
        let (exited, _) = watch::channel(false);
        let spawner = PipeSpawner(Mutex::new(Some(PipeProcess {
            stdin: Some(stdin),
            stdout: Some(stdout),
            exited: exited.clone(),
        })));
        let (agent, input) = mpsc::unbounded_channel();
        let (sent, frames) = mpsc::unbounded_channel();
        tokio::spawn(fake_agent(agent_reader, agent_writer, input, sent, exited));
        let (_level, level) = watch::channel(ora_logging::LogLevel::Info);
        let (plugin, mut notifications) = PluginRuntime::launch(
            &spawner,
            PluginRuntimeConfig {
                plugin_id: "official/test.agent".to_string(),
                deno_path: directory.path().join("unused-deno"),
                entrypoint,
                permissions: Vec::new(),
                cwd: None,
                environment: BTreeMap::new(),
                ready_timeout: Duration::from_secs(30),
                call_timeout: Duration::from_secs(30),
                shutdown_timeout: Duration::from_secs(1),
            },
            NoHostRequests,
            PluginLogSetup {
                root: directory.path().join("logs"),
                directory: directory.path().join("logs").join("agent"),
                host_session_id: "test".to_string(),
                generation: 1,
                level,
            },
        )
        .await
        .unwrap();
        let (messages, inbound) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(notification) = notifications.recv().await {
                if messages.send(Ok(notification.params)).is_err() {
                    break;
                }
            }
        });
        let (client, inbound) =
            AcpPeer::spawn(inbound, PluginAcpTransport::new(plugin.clone())).into_parts();
        let connection = RuntimeConnection {
            client: client.clone(),
            runtime: plugin.clone(),
            generation: 1,
            load_session_supported,
            http_mcp_supported: false,
            list_session_supported: false,
            close_session_supported: false,
            delete_session_supported: false,
        };
        let (state, receiver) = watch::channel(ConnectionState::Ready(connection));
        let (shutdown, mut shutdown_receiver) = mpsc::unbounded_channel();
        let routes = Arc::new(RouteRegistry::default());
        let active_generation = Arc::new(AtomicU64::new(1));
        let supervisor = ConnectionSupervisor {
            label: Arc::from("test"),
            state: receiver,
            active_generation: active_generation.clone(),
            routes: routes.clone(),
            shutdown,
        };
        let scheduler = ora_scheduler::Scheduler::new(chrono_tz::Asia::Shanghai);
        let runtime = test_runtime(
            directory.path(),
            Vec::new(),
            MemorySessionStore::default(),
            scheduler.clone(),
        );
        let host = runtime.manager.inner.connections.plugin_host.clone();
        let plugin_id = PluginId::parse("official/test.agent").unwrap();
        runtime
            .manager
            .inner
            .connections
            .supervisors
            .write()
            .unwrap()
            .insert(AgentRef::for_plugin(&plugin_id), supervisor.clone());
        let process_plugin = plugin.clone();
        let routed = tokio::spawn(async move {
            let mut process = startup::SharedProcess {
                process: AgentProcess {
                    plugin_id,
                    runtime: process_plugin,
                    host,
                },
                client,
                inbound,
                load_session_supported,
                http_mcp_supported: false,
                list_session_supported: false,
                close_session_supported: false,
                delete_session_supported: false,
            };
            run_process_generation(&mut process, &routes, &mut shutdown_receiver).await;
            active_generation.store(0, Ordering::Release);
            routes.fail_generation(1, runtime_unavailable_because("fake provider disconnected"));
        });
        Self {
            _directory: directory,
            manager: runtime.manager,
            supervisor,
            plugin,
            agent,
            frames,
            routed,
            state,
            scheduler,
        }
    }

    /// Persists the binding that survives a restart, independently of any live actor.
    fn seed_session(&self, id: &str) -> Session {
        self.manager
            .inner
            .store
            .create_session(Session::new(
                SessionId::new(id),
                WorkspaceId::new("workspace"),
                AgentRef::parse("official/test.agent").unwrap(),
                id,
                SessionStatus::Running,
                SessionMcpSelection::Automatic,
                AuditFields::new(
                    /*created_at*/ 0, /*updated_at*/ 0, /*is_deleted*/ false,
                ),
            ))
            .unwrap()
    }

    /// Seeds an already attached session so the test measures prompt policy rather than setup.
    fn add_session(&self, id: &str) {
        self.add_session_with_title_acquisition(id, crate::TitleAcquisition::disabled());
    }

    /// Retains the title lifecycle established by setup so title admission can also be exercised.
    fn add_session_with_title_acquisition(
        &self,
        id: &str,
        title_acquisition: crate::TitleAcquisition,
    ) {
        let session = self.seed_session(id);
        let recorder = self.manager.open_recorder(&session).unwrap().recorder;
        let channel = self.supervisor.open_session_channel(id, id).unwrap();
        self.manager
            .insert_actor(
                session,
                ActorSetup {
                    session_mcp: NoSessionMcp::default(),
                    cwd: self._directory.path().to_path_buf(),
                    connection: self.supervisor.clone(),
                    channel: Some(channel),
                    recorder,
                    handoff: crate::HandoffDebt::Settled,
                    title_acquisition,
                    live_mcp: LiveMcpState::Active(crate::SessionMcpRevision::default()),
                    config_options: Vec::new(),
                },
            )
            .unwrap();
    }

    /// Observes a frame emitted by the actual plugin transport, without reading actor internals.
    async fn next_frame(&mut self) -> Value {
        self.frames.recv().await.expect("agent frame")
    }

    /// Ends every actor and the provider while retaining normal ownership cleanup.
    async fn shutdown(self) {
        for id in self
            .manager
            .inner
            .store
            .list_sessions()
            .unwrap()
            .into_iter()
            .map(|session| session.id.to_string())
        {
            let _ = self
                .manager
                .stop_session(ora_contracts::StopSessionRequest { session_id: id })
                .await;
        }
        self.manager.inner.actors.write().unwrap().clear();
        self.plugin.shutdown_and_wait().await;
        self.routed.await.unwrap();
        self.scheduler.shutdown().await;
    }
}

/// A new prompt preempting a real in-flight title request must retain its explicit waiting policy.
#[test]
fn title_polling_admission_preserves_a_waiting_prompt_policy() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session_with_title_acquisition(
            "waiting",
            crate::TitleAcquisition::awaiting_first_prompt(/*list_supported*/ true),
        );
        let mut first = fixture
            .manager
            .prompt_session(prompt("waiting"))
            .await
            .unwrap();
        fixture.next_frame().await;
        fixture
            .agent
            .send(AgentInput::Finish(
                "waiting".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut first).await, StopReason::EndTurn);
        // The real scheduler starts the bounded fallback. Its unanswered request proves the
        // next admission enters title_polling's preemption branch rather than the idle loop.
        assert_eq!(fixture.next_frame().await["method"], "session/list");
        let mut waiting = fixture
            .manager
            .prompt_session_with_inactivity_policy(
                prompt("waiting"),
                PromptInactivityPolicy::Unbounded,
            )
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(7200)).await;
        assert!(waiting.try_recv().is_none());
        assert!(fixture.frames.try_recv().is_err());
        fixture
            .agent
            .send(AgentInput::Finish(
                "waiting".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut waiting).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// Exchanges plugin envelopes and ACP frames exactly as an external agent does.
async fn fake_agent(
    mut reader: DuplexStream,
    mut writer: DuplexStream,
    mut commands: mpsc::UnboundedReceiver<AgentInput>,
    frames: mpsc::UnboundedSender<Value>,
    exited: watch::Sender<bool>,
) {
    let registration = json!({"jsonrpc":"2.0","method":"ora/register","params":{"methods":[],"emits":["agent/acp"]}});
    write_message(&mut writer, &registration).await.unwrap();
    let (requests, mut received) = mpsc::unbounded_channel();
    // Framing stays owned by this reader while the agent processes commands; a control input
    // cannot cancel a partial read and accidentally treat body bytes as the next frame header.
    tokio::spawn(async move {
        while let Ok(Some(frame)) = read_message(&mut reader).await {
            if requests.send(frame).is_err() {
                break;
            }
        }
    });
    let mut pending = HashMap::new();
    let mut cancel_replies = HashMap::new();
    loop {
        let response = tokio::select! {
            envelope = received.recv() => {
                let Some(envelope) = envelope else { break; };
                if envelope["method"] == "ora/shutdown" { break; }
                let frame = envelope["params"].clone();
                let session = frame["params"]["sessionId"].as_str().unwrap_or_default().to_string();
                match frame["method"].as_str() {
                    Some("session/prompt") => { pending.insert(session, frame["id"].clone()); frames.send(frame).unwrap(); None }
                    Some("session/cancel") => {
                        frames.send(frame).unwrap();
                        match (pending.remove(&session), cancel_replies.remove(&session).unwrap_or(CancelReply::Confirm)) {
                            (Some(id), CancelReply::Confirm) => Some(json!({"jsonrpc":"2.0","id":id,"result":PromptResponse::new(StopReason::Cancelled)})),
                            (Some(id), CancelReply::EndTurn) => Some(json!({"jsonrpc":"2.0","id":id,"result":PromptResponse::new(StopReason::EndTurn)})),
                            (Some(id), CancelReply::RequestError) => Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"fake cancellation failure"}})),
                            (Some(id), CancelReply::Ignore) => { pending.insert(session, id); None }
                            (None, _) => None,
                        }
                    }
                    _ => { frames.send(frame).unwrap(); None }
                }
            }
            input = commands.recv() => match input {
                Some(AgentInput::Finish(session, reason)) => Some(json!({"jsonrpc":"2.0","id":pending.remove(&session).expect("pending prompt"),"result":PromptResponse::new(reason)})),
                Some(AgentInput::Update(update)) => Some(json!({"jsonrpc":"2.0","method":"session/update","params":update})),
                Some(AgentInput::RawFrame(frame)) => Some(frame),
                Some(AgentInput::Permission { session_id, tool_call }) => Some(json!({
                    "jsonrpc":"2.0",
                    "id": format!("permission-{session_id}"),
                    "method":"session/request_permission",
                    "params":{
                        "sessionId": session_id,
                        "toolCall": tool_call,
                        "options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"}],
                    },
                })),
                Some(AgentInput::Disconnect) | None => break,
                Some(AgentInput::CancelReply { session_id, reply, configured }) => {
                    cancel_replies.insert(session_id, reply);
                    configured.send(()).unwrap();
                    None
                }
            }
        };
        if let Some(response) = response {
            let envelope = json!({"jsonrpc":"2.0","method":"agent/acp","params":response});
            write_message(&mut writer, &envelope).await.unwrap();
        }
    }
    exited.send_replace(true);
}

/// Builds the same public request ordinary chat and workflow execution admit.
fn prompt(id: &str) -> PromptSessionRequest {
    PromptSessionRequest {
        session_id: id.to_string(),
        prompt: vec![ContentBlock::Text(TextContent::new(
            "delegate to an internal agent",
        ))],
        record_prompt: None,
        model: None,
    }
}

/// Runs production spans and events under one scoped TRACE subscriber and controllable clock.
fn run_case(test: impl Future<Output = ()>) {
    ora_logging::initialize_test_clock();
    ora_logging::with_trace_logging(|| block_on_paused_clock(test));
}

/// Drives a case on a controllable clock under whatever subscriber its caller installed.
fn block_on_paused_clock(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(/*start_paused*/ true)
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(86400), test)
                .await
                .expect("policy test must settle within one virtual day");
        });
}

/// Reads past ordinary setup/update traffic to assert a terminal production response.
async fn completion(stream: &mut SessionEventStream<PromptSessionEvent>) -> StopReason {
    while let Some(event) = stream.recv().await {
        if let PromptSessionEvent::Completed { stop_reason, .. } = event.unwrap() {
            return stop_reason;
        }
    }
    panic!("prompt ended without completion")
}

/// A hidden child can stay silent for hours without changing another session's default timer.
/// See specs/test-cases/desktop/core/workflow/prompt-inactivity.md#a-silent-waiting-prompt-remains-cancellable-and-session-local.
#[test]
fn unbounded_prompt_survives_silence_while_default_session_retries() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        fixture.add_session("default");
        let mut waiting = fixture
            .manager
            .prompt_session_with_inactivity_policy(
                prompt("waiting"),
                PromptInactivityPolicy::Unbounded,
            )
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["params"]["sessionId"], "waiting");
        let mut default = fixture
            .manager
            .prompt_session(prompt("default"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(45)).await;
        let cancelled = fixture.next_frame().await;
        assert_eq!(
            (
                cancelled["method"].clone(),
                cancelled["params"]["sessionId"].clone()
            ),
            (json!("session/cancel"), json!("default"))
        );
        assert_eq!(fixture.next_frame().await["params"]["sessionId"], "default");
        assert!(matches!(
            default.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying {
                retry: 1,
                max_retries: 3
            }
        ));
        fixture
            .agent
            .send(AgentInput::Finish(
                "default".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut default).await, StopReason::EndTurn);
        tokio::time::advance(Duration::from_secs(7200)).await;
        assert!(waiting.try_recv().is_none());
        assert!(fixture.frames.try_recv().is_err());
        fixture
            .agent
            .send(AgentInput::Finish(
                "waiting".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut waiting).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// The runtime names why a turn ended, because history records every early end as `Cancelled`.
///
/// See specs/test-cases/desktop/core/workflow/first-turn-confirmation.md.
#[test]
fn the_prompt_reports_who_ended_the_turn() {
    run_case(async {
        let mut fixture = Fixture::new().await;

        // The user cancels the active prompt from the session UI.
        fixture.add_session("stopped");
        let mut stopped = fixture
            .manager
            .prompt_session(prompt("stopped"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        fixture
            .manager
            .cancel_session_prompt(CancelSessionPromptRequest {
                session_id: "stopped".to_string(),
            })
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(completion(&mut stopped).await, StopReason::Cancelled);
        assert_eq!(stopped.end_cause(), Some(PromptEndCause::UserStopped));

        // The turn's owner abandons the stream without answering it.
        fixture.add_session("dropped");
        let mut dropped = fixture
            .manager
            .prompt_session(prompt("dropped"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        dropped.cancel_and_wait().await.unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(dropped.end_cause(), Some(PromptEndCause::StreamDropped));

        // The inactivity schedule runs out of retries while the provider stays silent.
        fixture.add_session("timed-out");
        let mut timed_out = fixture
            .manager
            .prompt_session(prompt("timed-out"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        let mut failure = None;
        while let Some(event) = timed_out.recv().await {
            if let Err(error) = event {
                failure = Some(error);
                break;
            }
        }
        assert!(
            failure.is_some(),
            "the exhausted policy reports its own timeout as a failure"
        );
        assert_eq!(
            timed_out.end_cause(),
            Some(PromptEndCause::InactivityTimeout)
        );

        fixture.shutdown().await;
    });
}

/// A provider that dies mid-turn is attributed to the session, not to a stream its owner dropped.
///
/// History records the end as `Cancelled`, so only the actor's report distinguishes a crash from
/// the owner abandoning the stream; this asserts that branch on a real provider EOF.
#[test]
fn a_provider_crash_reports_session_crashed_rather_than_a_dropped_stream() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("crashed");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("crashed"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        fixture.agent.send(AgentInput::Disconnect).unwrap();
        let error = stream.recv().await.unwrap().unwrap_err();
        assert_eq!(
            error.public_error(),
            &PublicError::AgentRuntimeUnavailable(EmptyErrorParams {})
        );
        assert_eq!(stream.end_cause(), Some(PromptEndCause::SessionCrashed));
        fixture.shutdown().await;
    });
}

/// Projected child events remain visible but cannot reset or pause the parent timeout.
#[test]
fn projected_child_activity_does_not_keep_an_unrelated_parent_prompt_alive() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("parent");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("parent"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(30)).await;
        for value in [
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"background"},"_meta":{"opencode/child-session":{"id":"child","parentID":"parent","depth":1}}}),
            json!({"sessionUpdate":"tool_call","toolCallId":"child:tool","title":"background","status":"in_progress","_meta":{"opencode/child-session":{"id":"child","parentID":"parent","depth":1}}}),
            json!({"sessionUpdate":"tool_call_update","toolCallId":"child:tool","status":"in_progress"}),
        ] {
            let notification: SessionNotification =
                serde_json::from_value(json!({"sessionId":"parent","update":value})).unwrap();
            fixture
                .agent
                .send(AgentInput::Update(notification))
                .unwrap();
            assert!(matches!(
                stream.recv().await.unwrap().unwrap(),
                PromptSessionEvent::SessionUpdate { .. }
            ));
        }
        tokio::time::advance(Duration::from_secs(15)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        assert!(matches!(
            stream.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        fixture
            .agent
            .send(AgentInput::Finish(
                "parent".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// A projected child permission round trip is routed but cannot rearm the parent window.
#[test]
fn projected_child_permission_does_not_rearm_the_parent_prompt() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("parent");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("parent"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(30)).await;
        fixture
            .agent
            .send(AgentInput::Permission {
                session_id: "parent".to_string(),
                tool_call: json!({
                    "toolCallId": "child:tool",
                    "_meta": {"opencode/child-session": {"id": "child", "parentID": "parent", "depth": 1}},
                }),
            })
            .unwrap();
        // The auto-allow response proves the request went through the production route.
        let allowed = fixture.next_frame().await;
        assert_eq!(allowed["id"], json!("permission-parent"));
        assert_eq!(
            allowed["result"]["outcome"]["optionId"],
            json!("allow-once")
        );
        // The actor settles the permission in the same task; letting it run once
        // keeps the timer advance below independent of scheduling order.
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(15)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        assert!(matches!(
            stream.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        fixture
            .agent
            .send(AgentInput::Finish(
                "parent".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// A parent tool's awaited permission still rearms the window through the routed actor.
#[test]
fn parent_permission_rearms_the_prompt_window() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("parent");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("parent"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(30)).await;
        fixture
            .agent
            .send(AgentInput::Permission {
                session_id: "parent".to_string(),
                tool_call: json!({"toolCallId": "delegate"}),
            })
            .unwrap();
        let allowed = fixture.next_frame().await;
        assert_eq!(
            allowed["result"]["outcome"]["optionId"],
            json!("allow-once")
        );
        tokio::task::yield_now().await;
        // The awaited parent permission rearmed the full window, so the original
        // deadline no longer cancels the prompt.
        tokio::time::advance(Duration::from_secs(15)).await;
        assert!(fixture.frames.try_recv().is_err());
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        assert!(matches!(
            stream.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        fixture
            .agent
            .send(AgentInput::Finish(
                "parent".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// Explicit cancellation still obtains the actual provider fence after an arbitrarily long wait.
#[test]
fn unbounded_prompt_remains_cancellable_and_reusable() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(
                prompt("waiting"),
                PromptInactivityPolicy::Unbounded,
            )
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(7200)).await;
        assert!(stream.try_recv().is_none());
        fixture
            .agent
            .send(AgentInput::Finish(
                "waiting".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);

        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(
                prompt("waiting"),
                PromptInactivityPolicy::Unbounded,
            )
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(7200)).await;
        stream.cancel_and_wait().await.unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        let mut next = fixture
            .manager
            .prompt_session(prompt("waiting"))
            .await
            .unwrap();
        fixture.next_frame().await;
        tokio::time::advance(Duration::from_secs(45)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        fixture.next_frame().await;
        assert!(matches!(
            next.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        next.cancel_and_wait().await.unwrap();
        fixture.shutdown().await;
    });
}

/// Dropping a silent owning stream sends cancellation without relying on provider updates.
#[test]
fn dropping_a_waiting_prompt_cancels_its_owner() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(
                prompt("waiting"),
                PromptInactivityPolicy::Unbounded,
            )
            .await
            .unwrap();
        fixture.next_frame().await;
        tokio::time::advance(Duration::from_secs(7200)).await;
        drop(stream);
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        // Stop queues behind the cancellation fence, proving the actor released the owner.
        fixture
            .manager
            .stop_session(ora_contracts::StopSessionRequest {
                session_id: "waiting".to_string(),
            })
            .await
            .unwrap();
        fixture.shutdown().await;
    });
}

/// A provider-confirmed timeout releases the caller only after the same session is reusable.
#[test]
fn confirmed_bounded_timeout_can_prompt_again_without_reloading() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(1800)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(
            stream.recv().await.unwrap().unwrap_err().public_error(),
            &PublicError::AgentTimedOut(EmptyErrorParams {})
        );
        let mut next = fixture
            .manager
            .prompt_session(prompt("waiting"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        fixture
            .agent
            .send(AgentInput::Finish("waiting".into(), StopReason::EndTurn))
            .unwrap();
        assert_eq!(completion(&mut next).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// A queued progress frame keeps the clock it received at ingress while the actor was busy.
#[test]
fn wait_uses_the_routed_arrival_time_even_when_processed_after_the_deadline() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(1799)).await;
        let arrived = tokio::time::Instant::now();
        let update = serde_json::from_value(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"progress"}})).unwrap();
        fixture
            .supervisor
            .routes
            .route_event(crate::routing::SessionEvent::Update(
                SessionNotification::new("waiting", update),
                arrived,
            ))
            .unwrap();
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(matches!(
            stream.recv().await.unwrap().unwrap(),
            PromptSessionEvent::SessionUpdate { .. }
        ));
        let before_expiry = arrived + Duration::from_secs(1800) - Duration::from_millis(1);
        assert!(
            tokio::time::timeout_at(before_expiry, fixture.next_frame())
                .await
                .is_err(),
            "a timely progress frame must prevent an early cancel"
        );
        tokio::time::advance(Duration::from_millis(1)).await;
        let cancel = tokio::time::timeout(Duration::from_secs(1), fixture.next_frame())
            .await
            .expect("expiry follows receipt, including time spent queued");
        assert_eq!(cancel["method"], "session/cancel");
        assert!(stream.recv().await.unwrap().is_err());
        fixture.shutdown().await;
    });
}

/// Queue an inert permission in the fixture's in-memory route without yielding to its actor.
fn queue_fixture_permission(fixture: &Fixture) {
    let request = ora_acp::PermissionRequest {
        request_id: serde_json::from_value(json!("pending")).unwrap(),
        request: serde_json::from_value(json!({
            "sessionId":"waiting","toolCall":{"toolCallId":"fixture"},
            "options":[{"optionId":"once","name":"Fixture choice","kind":"allow_once"}]
        }))
        .unwrap(),
    };
    fixture
        .supervisor
        .routes
        .route_event(crate::routing::SessionEvent::Permission(request))
        .unwrap();
}

/// An outstanding permission belongs to cancellation when its window has already closed.
#[test]
fn bounded_settlement_cancels_a_pending_permission_at_the_boundary() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(1799)).await;
        queue_fixture_permission(&fixture);
        tokio::time::advance(Duration::from_secs(1)).await;
        let reply = fixture.next_frame().await;
        assert_eq!(reply["id"], "pending");
        assert_eq!(reply["result"]["outcome"]["outcome"], "cancelled");
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(
            stream.recv().await.unwrap().unwrap_err().public_error(),
            &PublicError::AgentTimedOut(EmptyErrorParams {})
        );
        fixture.shutdown().await;
    });
}

/// Timely progress in the boundary batch lets a temporarily deferred request finish once.
#[test]
fn renewed_wait_resumes_a_deferred_permission() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(1799)).await;
        queue_fixture_permission(&fixture);
        let progress = serde_json::from_value(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"continuing"}})).unwrap();
        fixture
            .supervisor
            .routes
            .route_event(crate::routing::SessionEvent::update(
                SessionNotification::new("waiting", progress),
            ))
            .unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        let reply = fixture.next_frame().await;
        assert_eq!(reply["id"], "pending");
        assert_eq!(reply["result"]["outcome"]["optionId"], "once");
        fixture
            .agent
            .send(AgentInput::Finish(
                "waiting".to_owned(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
        assert!(fixture.frames.try_recv().is_err());
        fixture.shutdown().await;
    });
}

/// A ready provider completion wins the whole batch, including a deferred request with no selectable option.
#[test]
fn renewed_wait_settles_a_ready_terminal_before_deferred_work() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        tokio::time::advance(Duration::from_secs(1799)).await;
        let terminal = crate::routing::tests::session_response("waiting").await;
        let request = ora_acp::PermissionRequest {
            request_id: serde_json::from_value(json!("inert")).unwrap(),
            request: serde_json::from_value(
                json!({"sessionId":"waiting","toolCall":{"toolCallId":"fixture"},"options":[]}),
            )
            .unwrap(),
        };
        fixture
            .supervisor
            .routes
            .route_event(crate::routing::SessionEvent::Permission(request))
            .unwrap();
        let progress = serde_json::from_value(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"continuing"}})).unwrap();
        fixture
            .supervisor
            .routes
            .route_event(crate::routing::SessionEvent::update(
                SessionNotification::new("waiting", progress),
            ))
            .unwrap();
        fixture
            .supervisor
            .routes
            .route_event(crate::routing::SessionEvent::Response(terminal))
            .unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        let reply = fixture.next_frame().await;
        assert_eq!(reply["id"], "inert");
        assert_eq!(reply["result"]["outcome"]["outcome"], "cancelled");
        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
        assert!(fixture.frames.try_recv().is_err());
        fixture.shutdown().await;
    });
}

/// Traffic already queued behind a non-cancel response is recorded before the old route closes.
#[test]
fn unconfirmed_wait_drains_the_queued_tail_before_failure() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        let mut follower = fixture
            .manager
            .load_session(ora_contracts::LoadSessionRequest {
                session_id: "waiting".into(),
            })
            .await
            .unwrap();
        let (configured, done) = oneshot::channel();
        fixture
            .agent
            .send(AgentInput::CancelReply {
                session_id: "waiting".to_owned(),
                reply: CancelReply::Ignore,
                configured,
            })
            .unwrap();
        done.await.unwrap();
        let terminal = crate::routing::tests::session_response("waiting").await;
        tokio::time::advance(Duration::from_secs(1800)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        let routes = &fixture.supervisor.routes;
        let settling: SessionUpdate = serde_json::from_value(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"settling"}})).unwrap();
        routes
            .route_event(crate::routing::SessionEvent::update(
                SessionNotification::new("waiting", settling.clone()),
            ))
            .unwrap();
        // Settlement consumes the first update and the response. The drain consumes the tail.
        routes
            .route_event(crate::routing::SessionEvent::Response(terminal))
            .unwrap();
        let tail: SessionUpdate = serde_json::from_value(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"last update"}})).unwrap();
        routes
            .route_event(crate::routing::SessionEvent::update(
                SessionNotification::new("waiting", tail.clone()),
            ))
            .unwrap();
        assert!(
            matches!(stream.recv().await.unwrap().unwrap(), PromptSessionEvent::SessionUpdate { update, .. } if update == settling)
        );
        assert!(
            matches!(stream.recv().await.unwrap().unwrap(), PromptSessionEvent::SessionUpdate { update, .. } if update == tail)
        );
        assert_eq!(
            stream.recv().await.unwrap().unwrap_err().public_error(),
            &PublicError::AgentTimedOut(EmptyErrorParams {})
        );
        assert!(stream.recv().await.is_none());
        let mut received = Vec::new();
        loop {
            match follower
                .recv()
                .await
                .expect("follower error after the boundary")
                .unwrap()
            {
                ora_contracts::LoadSessionEvent::SessionUpdate { update, .. }
                    if update == tail || update == settling =>
                {
                    received.push(update)
                }
                ora_contracts::LoadSessionEvent::TurnEnded {
                    stop_reason: StopReason::Cancelled,
                    ..
                } => break,
                _ => {}
            }
        }
        assert_eq!(received, vec![settling, tail]);
        assert_eq!(
            follower.recv().await.unwrap().unwrap_err().public_error(),
            &PublicError::AgentTimedOut(EmptyErrorParams {})
        );
        assert!(follower.recv().await.is_none());
        fixture.shutdown().await;
    });
}

/// Expiry settles the owner even if the CLI also ignores cancellation.
#[test]
fn waiting_prompt_times_out_without_a_cancellation_acknowledgement() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        let mut connection = fixture.supervisor.current().unwrap();
        connection.close_session_supported = true;
        fixture
            .state
            .send_replace(ConnectionState::Ready(connection));
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        let mut follower = fixture
            .manager
            .load_session(ora_contracts::LoadSessionRequest {
                session_id: "waiting".into(),
            })
            .await
            .unwrap();
        let (configured, done) = oneshot::channel();
        fixture
            .agent
            .send(AgentInput::CancelReply {
                session_id: "waiting".into(),
                reply: CancelReply::Ignore,
                configured,
            })
            .unwrap();
        done.await.unwrap();
        tokio::time::advance(Duration::from_secs(1799)).await;
        assert!(stream.try_recv().is_none());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        fixture
            .manager
            .cancel_turn_prompt(
                "waiting",
                tokio::time::Instant::now() + Duration::from_secs(60),
            )
            .unwrap();
        tokio::time::advance(crate::limits::CANCELLATION_GRACE - Duration::from_millis(1)).await;
        assert!(
            stream.try_recv().is_none(),
            "the node must wait for the outstanding cancellation"
        );
        let close = fixture.next_frame().await;
        assert_eq!(close["method"], "session/close");
        assert!(
            stream.try_recv().is_none(),
            "the node must wait for isolation to settle"
        );
        fixture
            .agent
            .send(AgentInput::RawFrame(
                json!({"jsonrpc":"2.0","id":close["id"],"result":{}}),
            ))
            .unwrap();
        let err = stream.recv().await.unwrap().unwrap_err();
        assert!(!err.is_turn_cancel_unanswered());
        assert_eq!(
            err.public_error(),
            &PublicError::AgentTimedOut(EmptyErrorParams {})
        );
        assert!(stream.recv().await.is_none());
        let mut ended = false;
        loop {
            let event = follower
                .recv()
                .await
                .expect("the timeout must reach the follower");
            match event {
                Ok(ora_contracts::LoadSessionEvent::TurnEnded {
                    stop_reason: StopReason::Cancelled,
                    ..
                }) => {
                    assert!(!ended);
                    ended = true;
                }
                Err(error) => {
                    assert!(
                        ended,
                        "the follower must receive the turn boundary before its error"
                    );
                    assert_eq!(error.public_error(), err.public_error());
                    break;
                }
                Ok(ora_contracts::LoadSessionEvent::Completed) => {
                    panic!("a timed-out follower cannot complete successfully")
                }
                Ok(_) => {}
            }
        }
        assert!(ended);
        assert!(follower.recv().await.is_none());
        let late = SessionNotification::new(
            "waiting",
            SessionUpdate::Plan(agent_client_protocol_schema::v1::Plan::new(Vec::new())),
        );
        assert!(
            fixture
                .supervisor
                .routes
                .route_event(crate::routing::SessionEvent::update(late))
                .is_err(),
            "the old route must be detached before the owner sees failure"
        );
        assert_eq!(stream.end_cause(), Some(PromptEndCause::InactivityTimeout));
        assert!(fixture.frames.try_recv().is_err());
        fixture.shutdown().await;
    });
}

/// A real provider EOF terminates an opted-out prompt through the generation's failure control.
#[test]
fn provider_disconnect_ends_an_unbounded_prompt() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(
                prompt("waiting"),
                PromptInactivityPolicy::Unbounded,
            )
            .await
            .unwrap();
        fixture.next_frame().await;
        fixture.agent.send(AgentInput::Disconnect).unwrap();
        let error = stream.recv().await.unwrap().unwrap_err();
        assert_eq!(
            error.public_error(),
            &ora_contracts::PublicError::AgentRuntimeUnavailable(
                ora_contracts::EmptyErrorParams {}
            )
        );
        fixture.shutdown().await;
    });
}

/// Releasing the host and its active stream cancels a silent owner before the router shuts down.
#[test]
fn releasing_the_host_and_stream_ends_a_silent_waiting_actor() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(
                prompt("waiting"),
                PromptInactivityPolicy::Unbounded,
            )
            .await
            .unwrap();
        fixture.next_frame().await;
        tokio::time::advance(Duration::from_secs(7200)).await;
        let Fixture {
            manager,
            supervisor,
            plugin,
            mut frames,
            routed,
            scheduler,
            _directory,
            state,
            agent,
        } = fixture;
        drop(manager);
        drop(supervisor);
        drop(stream);
        assert_eq!(frames.recv().await.unwrap()["method"], "session/cancel");
        // The generation task can finish only after the actor released the final supervisor.
        routed.await.unwrap();
        plugin.shutdown_and_wait().await;
        scheduler.shutdown().await;
        drop((agent, state, _directory));
    });
}

/// Provider lifecycle can already protect a delegated parent without disabling its default timer.
#[test]
fn running_delegate_pauses_default_policy_until_it_finishes() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("parent");
        let mut parent = fixture
            .manager
            .prompt_session(prompt("parent"))
            .await
            .unwrap();
        fixture.next_frame().await;
        for status in [
            agent_client_protocol_schema::v1::ToolCallStatus::InProgress,
            agent_client_protocol_schema::v1::ToolCallStatus::Completed,
        ] {
            fixture
                .agent
                .send(AgentInput::Update(SessionNotification::new(
                    "parent",
                    SessionUpdate::ToolCall(
                        agent_client_protocol_schema::v1::ToolCall::new("delegate", "Explore")
                            .status(status),
                    ),
                )))
                .unwrap();
            parent.recv().await.unwrap().unwrap();
            if status == agent_client_protocol_schema::v1::ToolCallStatus::InProgress {
                tokio::time::advance(Duration::from_secs(7200)).await;
                assert!(parent.try_recv().is_none());
                assert!(fixture.frames.try_recv().is_err());
            }
        }
        tokio::time::advance(Duration::from_secs(45)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        fixture.next_frame().await;
        assert!(matches!(
            parent.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        parent.cancel_and_wait().await.unwrap();
        fixture.shutdown().await;
    });
}

/// Republishing a running delegate with an explicit null keeps its pause and still settles it.
#[test]
fn cleared_child_source_keeps_the_delegate_pause_and_settles_on_completion() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("parent");
        let mut parent = fixture
            .manager
            .prompt_session(prompt("parent"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        for value in [
            json!({"sessionUpdate":"tool_call","toolCallId":"delegate","title":"Explore","status":"in_progress"}),
            // Late tool metadata replaces the namespace; an explicit null names no child.
            json!({"sessionUpdate":"tool_call_update","toolCallId":"delegate","status":"in_progress","_meta":{"opencode/child-session":null}}),
        ] {
            let notification: SessionNotification =
                serde_json::from_value(json!({"sessionId":"parent","update":value})).unwrap();
            fixture
                .agent
                .send(AgentInput::Update(notification))
                .unwrap();
            parent.recv().await.unwrap().unwrap();
        }

        // The delegate is still the parent's, so the default timer cannot cut it short.
        // Draining the scheduler proves no retry is merely waiting for its next poll.
        tokio::time::advance(Duration::from_secs(7200)).await;
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert!(parent.try_recv().is_none());
        assert!(fixture.frames.try_recv().is_err());

        // Its completion still settles the prompt and restarts the ordinary window.
        let notification: SessionNotification = serde_json::from_value(json!({
            "sessionId":"parent",
            "update":{"sessionUpdate":"tool_call_update","toolCallId":"delegate","status":"completed"}
        }))
        .unwrap();
        fixture
            .agent
            .send(AgentInput::Update(notification))
            .unwrap();
        parent.recv().await.unwrap().unwrap();
        tokio::time::advance(Duration::from_secs(45)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        assert!(matches!(
            parent.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        parent.cancel_and_wait().await.unwrap();
        fixture.shutdown().await;
    });
}

/// The unchanged default exhausts precisely three confirmed retries and then isolates its session.
#[test]
fn default_policy_exhausts_three_retries() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("default");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("default"))
            .await
            .unwrap();
        fixture.next_frame().await;
        for (retry, window) in [(1, 45), (2, 60), (3, 90)] {
            tokio::time::advance(Duration::from_secs(window)).await;
            assert_eq!(fixture.next_frame().await["method"], "session/cancel");
            assert_eq!(fixture.next_frame().await["method"], "session/prompt");
            assert!(
                matches!(stream.recv().await.unwrap().unwrap(), PromptSessionEvent::Retrying { retry: actual, max_retries: 3 } if actual == retry)
            );
        }
        tokio::time::advance(Duration::from_secs(120)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        let error = stream.recv().await.unwrap().unwrap_err();
        assert_eq!(
            error.public_error(),
            &ora_contracts::PublicError::AgentTimedOut(ora_contracts::EmptyErrorParams {})
        );
        assert!(fixture.frames.try_recv().is_err());
        fixture.shutdown().await;
    });
}

/// Parent Pending does not claim a running tool; unrelated child updates cannot refresh it.
#[test]
fn child_updates_do_not_rearm_the_parent_prompt() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("parent");
        fixture.add_session("child");
        let mut parent = fixture
            .manager
            .prompt_session(prompt("parent"))
            .await
            .unwrap();
        fixture.next_frame().await;
        let mut child = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("child"), PromptInactivityPolicy::Wait)
            .await
            .unwrap();
        fixture.next_frame().await;
        fixture
            .agent
            .send(AgentInput::Update(SessionNotification::new(
                "parent",
                SessionUpdate::ToolCall(
                    agent_client_protocol_schema::v1::ToolCall::new("delegate", "Explore")
                        .status(agent_client_protocol_schema::v1::ToolCallStatus::Pending),
                ),
            )))
            .unwrap();
        parent.recv().await.unwrap().unwrap();
        tokio::time::advance(Duration::from_secs(30)).await;
        fixture
            .agent
            .send(AgentInput::Update(SessionNotification::new(
                "child",
                SessionUpdate::AgentMessageChunk(
                    agent_client_protocol_schema::v1::ContentChunk::new(ContentBlock::Text(
                        TextContent::new("still exploring"),
                    )),
                ),
            )))
            .unwrap();
        child.recv().await.unwrap().unwrap();
        tokio::time::advance(Duration::from_secs(15)).await;
        assert_eq!(fixture.next_frame().await["params"]["sessionId"], "parent");
        fixture.next_frame().await;
        assert!(matches!(
            parent.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        parent.cancel_and_wait().await.unwrap();
        child.cancel_and_wait().await.unwrap();
        fixture.shutdown().await;
    });
}

/// Missing, failed, and non-cancelled fences fail without a fresh provider turn.
#[test]
fn unconfirmed_cancellation_fences_never_resend_the_prompt() {
    run_case(async {
        for reply in [
            CancelReply::EndTurn,
            CancelReply::RequestError,
            CancelReply::Ignore,
        ] {
            let mut fixture = Fixture::new().await;
            fixture.add_session("default");
            let (configured, confirmed) = oneshot::channel();
            fixture
                .agent
                .send(AgentInput::CancelReply {
                    session_id: "default".to_string(),
                    reply,
                    configured,
                })
                .unwrap();
            confirmed.await.unwrap();
            let mut stream = fixture
                .manager
                .prompt_session(prompt("default"))
                .await
                .unwrap();
            fixture.next_frame().await;
            tokio::time::advance(Duration::from_secs(45)).await;
            assert_eq!(fixture.next_frame().await["method"], "session/cancel");
            let error = stream.recv().await.unwrap().unwrap_err();
            assert_eq!(
                error.public_error(),
                &ora_contracts::PublicError::AgentTimedOut(ora_contracts::EmptyErrorParams {})
            );
            assert!(fixture.frames.try_recv().is_err());
            fixture.shutdown().await;
        }
    });
}

/// Collects only content-free inactivity decisions with their original structured field types.
#[derive(Clone, Default)]
struct InactivityLogs(Arc<Mutex<Vec<BTreeMap<String, Value>>>>);

impl<S: tracing::Subscriber> Layer<S> for InactivityLogs {
    /// Captures production decisions under the test's isolated TRACE subscriber.
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let mut fields = LogFields(BTreeMap::new());
        event.record(&mut fields);
        if matches!(
            fields.0.get("message").and_then(Value::as_str),
            Some(
                "prompt inactive; cancelling stalled attempt"
                    | "inactive prompt cancellation settled"
                    | "workflow prompt exceeded its silence limit"
            )
        ) {
            fields.0.insert(
                "level".to_string(),
                json!(event.metadata().level().as_str()),
            );
            self.0.lock().unwrap().push(fields.0);
        }
    }
}

/// Preserves numeric durations and enum values without depending on a text formatter.
struct LogFields(BTreeMap<String, Value>);

impl Visit for LogFields {
    /// Records text without formatter-dependent quoting.
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_string(), json!(value));
    }
    /// Retains integer tool counts and attempt numbers.
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_string(), json!(value));
    }
    /// Retains production monotonic durations.
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().to_string(), json!(value));
    }
    /// Records typed policy and stop reason without serializing prompt or tool payloads.
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), json!(format!("{value:?}")));
    }
}

/// The bounded timeout has one structured, content-free record for the operation.
#[test]
fn bounded_timeout_logs_the_window_and_settlement_outcome() {
    let logs = InactivityLogs::default();
    ora_logging::initialize_test_clock();
    ora_logging::with_recorded_trace_logging(logs.clone(), || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(/*start_paused*/ true)
            .build()
            .unwrap()
            .block_on(async {
                let mut fixture = Fixture::new().await;
                fixture.add_session("waiting");
                let mut stream = fixture
                    .manager
                    .prompt_session_with_inactivity_policy(
                        prompt("waiting"),
                        PromptInactivityPolicy::Wait,
                    )
                    .await
                    .unwrap();
                assert_eq!(fixture.next_frame().await["method"], "session/prompt");
                tokio::time::advance(Duration::from_secs(1800)).await;
                assert!(stream.recv().await.unwrap().is_err());
                fixture.shutdown().await;
            });
    });
    let records = logs.0.lock().unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert!(record["operation_id"].as_u64().is_some_and(|id| id > 0));
    assert_eq!(
        serde_json::to_value(record).unwrap(),
        json!({
            "message":"workflow prompt exceeded its silence limit", "level":"WARN", "method":"finish_expired_wait",
            "session_id":"waiting", "operation_id":record["operation_id"], "policy":"Wait", "attempt":1,
            "window_ms":1800000.0, "silent_elapsed_ms":1800000.0, "running_tools":0, "pending_tools":0,
            "end_cause":"inactivity_timeout", "cancellation_result":"confirmed"
        })
    );
}

/// Diagnostics independently identify the inactivity decision and provider cancellation fence.
#[test]
fn inactivity_diagnostics_record_the_window_tools_and_cancellation_result() {
    let logs = InactivityLogs::default();
    ora_logging::initialize_test_clock();
    ora_logging::with_recorded_trace_logging(logs.clone(), || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(/*start_paused*/ true)
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(86400), async {
                    let mut fixture = Fixture::new().await;
                    fixture.add_session("default");
                    let mut request = prompt("default");
                    request.prompt = vec![ContentBlock::Text(TextContent::new(
                        "private-prompt-sentinel",
                    ))];
                    let mut stream = fixture.manager.prompt_session(request).await.unwrap();
                    fixture.next_frame().await;
                    fixture
                        .agent
                        .send(AgentInput::Update(SessionNotification::new(
                            "default",
                            SessionUpdate::ToolCall(
                                agent_client_protocol_schema::v1::ToolCall::new(
                                    "secret-tool-id",
                                    "private-tool-sentinel",
                                )
                                .status(agent_client_protocol_schema::v1::ToolCallStatus::Pending),
                            ),
                        )))
                        .unwrap();
                    stream.recv().await.unwrap().unwrap();
                    tokio::time::advance(Duration::from_secs(45)).await;
                    fixture.next_frame().await;
                    fixture.next_frame().await;
                    assert!(matches!(
                        stream.recv().await.unwrap().unwrap(),
                        PromptSessionEvent::Retrying { retry: 1, .. }
                    ));
                    stream.cancel_and_wait().await.unwrap();
                    fixture.shutdown().await;
                })
                .await
                .expect("diagnostic test must settle within one virtual day");
            });
    });
    let recorded = logs.0.lock().unwrap().clone();
    assert_eq!(
        recorded,
        vec![
            BTreeMap::from([
                ("session_id".to_string(), json!("default")),
                ("policy".to_string(), json!("Timeout")),
                ("attempt".to_string(), json!(1)),
                ("window_ms".to_string(), json!(45000.0)),
                ("silent_elapsed_ms".to_string(), json!(45000.0)),
                ("running_tools".to_string(), json!(0)),
                ("pending_tools".to_string(), json!(1)),
                ("method".to_string(), json!("retry_stalled_prompt")),
                ("level".to_string(), json!("WARN")),
                (
                    "message".to_string(),
                    json!("prompt inactive; cancelling stalled attempt")
                ),
            ]),
            BTreeMap::from([
                ("session_id".to_string(), json!("default")),
                ("policy".to_string(), json!("Timeout")),
                ("attempt".to_string(), json!(1)),
                ("cancellation_result".to_string(), json!("response")),
                ("method".to_string(), json!("retry_stalled_prompt")),
                ("stop_reason".to_string(), json!("Some(Cancelled)")),
                ("level".to_string(), json!("WARN")),
                (
                    "message".to_string(),
                    json!("inactive prompt cancellation settled")
                ),
            ]),
        ]
    );
}

/// `_subagent/update` frames flow through the real ACP peer and `run_process_generation` router
/// in wire order beside `session/update` frames and reach the targeted live session actor.
#[test]
fn routes_subagent_update_extensions_in_wire_order_through_process_generation() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("main-session");
        fixture.add_session("other-session");

        let mut stream = fixture
            .manager
            .prompt_session(prompt("main-session"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");

        for frame in [
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": "main-session",
                    "update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": "call_1",
                        "title": "Agent",
                        "status": "in_progress",
                    },
                },
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "_subagent/update",
                "params": {
                    "sessionId": "main-session",
                    "taskId": "task-explore",
                    "toolCallId": "call_1",
                    "agentType": "Explore",
                    "update": {
                        "sessionUpdate": "tool_call",
                        "toolCallId": "call_1",
                        "title": "Read",
                        "status": "in_progress",
                    },
                    "timestamp": 1001,
                },
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "_subagent/update",
                "params": {
                    "sessionId": "main-session",
                    "taskId": "task-explore",
                    "update": {
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "call_1",
                        "status": "completed",
                        "rawOutput": "ok",
                    },
                    "timestamp": 1002,
                },
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": "main-session",
                    "update": {
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "call_1",
                        "status": "completed",
                        "rawOutput": "Subagent summary",
                    },
                },
            }),
        ] {
            fixture.agent.send(AgentInput::RawFrame(frame)).unwrap();
        }
        fixture
            .agent
            .send(AgentInput::Finish(
                "main-session".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();

        let mut observed = Vec::new();
        while let Some(event) = stream.recv().await {
            if let Ok(PromptSessionEvent::SessionUpdate { update, .. }) = event {
                observed.push(update);
            }
        }
        assert_eq!(observed.len(), 4);
        assert_eq!(
            observed,
            vec![
                serde_json::from_value::<SessionUpdate>(json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "call_1",
                    "title": "Agent",
                    "status": "in_progress",
                }))
                .unwrap(),
                serde_json::from_value::<SessionUpdate>(json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "subagent:12:task-explore:call_1",
                    "title": "Read",
                    "status": "in_progress",
                    "_meta": {
                        "_subagent/update": {
                            "sessionId": "main-session",
                            "taskId": "task-explore",
                            "toolCallId": "call_1",
                            "agentType": "Explore",
                        }
                    },
                }))
                .unwrap(),
                serde_json::from_value::<SessionUpdate>(json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "subagent:12:task-explore:call_1",
                    "status": "completed",
                    "rawOutput": "ok",
                    "_meta": {
                        "_subagent/update": {
                            "sessionId": "main-session",
                            "taskId": "task-explore",
                            "toolCallId": "call_1",
                            "agentType": "Explore",
                        }
                    },
                }))
                .unwrap(),
                serde_json::from_value::<SessionUpdate>(json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "call_1",
                    "status": "completed",
                    "rawOutput": "Subagent summary",
                }))
                .unwrap(),
            ]
        );
        fixture.shutdown().await;
    });
}

/// Parses the capture's `HH:MM:SS.mmm` time of day into milliseconds; the s1 capture is one day.
fn captured_millis(at: &str) -> u64 {
    let time = at
        .split('T')
        .nth(1)
        .expect("timestamp")
        .trim_end_matches('Z');
    let (hms, ms) = time.split_once('.').expect("millis");
    let mut parts = hms
        .split(':')
        .map(|part| part.parse::<u64>().expect("clock part"));
    let hours = parts.next().expect("hours");
    let minutes = parts.next().expect("minutes");
    let seconds = parts.next().expect("seconds");
    (hours * 3600 + minutes * 60 + seconds) * 1000 + ms.parse::<u64>().expect("millis")
}

/// Loads the redacted real CodeAgent s1 capture (specs `20261118` D6 evidence): the update frames
/// after `session/prompt` with their real offsets, plus the captured response offset. The first
/// fixture line anchors the offsets at the captured prompt send; the fixture session is `sess-1`
/// so the captured frames route verbatim.
fn s1_real_replay() -> (Vec<(Duration, Value)>, Duration) {
    let lines: Vec<Value> = include_str!("prompt_policy_tests/s1_complete.jsonl")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture line"))
        .collect();
    let anchor = captured_millis(lines[0]["capturedAt"].as_str().expect("anchor"));
    let mut updates = Vec::new();
    let mut response_offset = None;
    for line in &lines[1..] {
        let offset = Duration::from_millis(
            captured_millis(line["capturedAt"].as_str().expect("timestamp")) - anchor,
        );
        if line["frame"]["method"].is_string() {
            updates.push((offset, line["frame"].clone()));
        } else {
            assert_eq!(line["frame"]["result"]["stopReason"], json!("end_turn"));
            response_offset = Some(offset);
        }
    }
    (updates, response_offset.expect("captured response"))
}

/// The captured s1 parent `Task` opening: `pending`, and no `in_progress` ever follows.
fn s1_parent_open() -> SessionNotification {
    serde_json::from_value(json!({
        "sessionId": "sess-1",
        "update": {
            "sessionUpdate": "tool_call",
            "toolCallId": "call-1",
            "title": "Task",
            "kind": "think",
            "status": "pending",
            "rawInput": {},
            "_meta": { "codeAgentCLI": { "toolName": "Agent" } },
        },
    }))
    .unwrap()
}

/// One real-shape `_subagent/update` extension frame pointing at the s1 parent delegation.
fn s1_subagent_frame(update: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "_subagent/update",
        "params": {
            "sessionId": "sess-1",
            "taskId": "task-1",
            "toolCallId": "call-1",
            "agentType": "Explore",
            "update": update,
            "timestamp": 1795073462666_u64,
        },
    })
}

/// Sends one captured `session/update` or `_subagent/update` frame through the peer/router and
/// waits until the actor emits its `SessionUpdate` so liveness observes it at the caller's clock.
async fn send_s1_frame(
    agent: &mpsc::UnboundedSender<AgentInput>,
    stream: &mut SessionEventStream<PromptSessionEvent>,
    frame: &Value,
) {
    match frame["method"].as_str().expect("method") {
        "session/update" => {
            let notification: SessionNotification =
                serde_json::from_value(frame["params"].clone()).expect("captured update");
            agent.send(AgentInput::Update(notification)).unwrap();
        }
        "_subagent/update" => {
            agent.send(AgentInput::RawFrame(frame.clone())).unwrap();
        }
        other => panic!("unexpected replay method {other}"),
    }
    match stream.recv().await.expect("stream open").expect("no error") {
        PromptSessionEvent::SessionUpdate { .. } => {}
        other => panic!("unexpected prompt event during s1 replay: {other:?}"),
    }
}

/// D6 evidence on the paused clock with production `PROMPT_INACTIVITY_WINDOWS`: the captured s1
/// delegation keeps the parent `Task` pending for 62.66 s while child frames arrive at uncompressed
/// real intervals (16.265 / 19.617 / 3.252 / 0.861 / 14.472 / 6.490 / 1.702 s); renewal must keep
/// the turn alive until the provider answers.
#[test]
fn real_s1_intervals_never_cancel_the_parent_prompt_under_default_timeout() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("sess-1");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("sess-1"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");

        let (updates, response_offset) = s1_real_replay();
        let start = tokio::time::Instant::now();
        for (offset, frame) in &updates {
            tokio::time::sleep_until(start + *offset).await;
            send_s1_frame(&fixture.agent, &mut stream, frame).await;
        }
        tokio::time::sleep_until(start + response_offset).await;
        fixture
            .agent
            .send(AgentInput::Finish(
                "sess-1".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();

        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
        while let Ok(frame) = fixture.frames.try_recv() {
            assert_ne!(frame["method"], json!("session/cancel"));
        }
        fixture.shutdown().await;
    });
}

/// Negative control on the exact `s1_complete` timeline: when `_subagent/update` frames carry an
/// unrecognized outer `toolCallId`, they do not renew the parent `Task` and the default 45 s
/// production window cancels the turn 45 s after the parent `Task` entered `pending`.
#[test]
fn s1_timeline_with_unknown_outer_id_cancels_after_45s() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("sess-1");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("sess-1"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");

        let (updates, _) = s1_real_replay();
        let start = tokio::time::Instant::now();
        let mut parent_pending_at = None;
        let mut cancelled_at = None;
        for (offset, mut frame) in updates {
            if frame["method"] == "_subagent/update" {
                frame["params"]["toolCallId"] = json!("unknown-call");
            }
            tokio::select! {
                () = tokio::time::sleep_until(start + offset) => {}
                outbound = fixture.frames.recv() => {
                    let outbound = outbound.expect("cancel frame");
                    assert_eq!(outbound["method"], json!("session/cancel"));
                    cancelled_at = Some(tokio::time::Instant::now());
                    break;
                }
            }
            send_s1_frame(&fixture.agent, &mut stream, &frame).await;
            if frame["method"] == "session/update"
                && frame["params"]["update"]["sessionUpdate"] == "tool_call"
                && frame["params"]["update"]["toolCallId"] == "call-1"
            {
                parent_pending_at = Some(tokio::time::Instant::now());
            }
        }
        let elapsed = cancelled_at
            .expect("unknown outer toolCallId must cancel on the first 45 s window")
            .duration_since(parent_pending_at.expect("parent Task opening observed"));
        assert_eq!(elapsed, Duration::from_secs(45));
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        assert!(matches!(
            stream.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        fixture
            .agent
            .send(AgentInput::Finish(
                "sess-1".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// D6 renewal is not a pause: after the last child frame a full silent window still cancels and
/// retries, so a stuck subagent cannot hold the parent prompt open indefinitely.
#[test]
fn a_silent_subagent_still_times_out_after_renewing_the_parent_window() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("sess-1");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("sess-1"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        fixture
            .agent
            .send(AgentInput::Update(s1_parent_open()))
            .unwrap();
        stream.recv().await.unwrap().unwrap();

        // One child frame 30 s into the first window rearms a full 45 s window.
        tokio::time::advance(Duration::from_secs(30)).await;
        fixture
            .agent
            .send(AgentInput::RawFrame(s1_subagent_frame(json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "still exploring" },
            }))))
            .unwrap();
        stream.recv().await.unwrap().unwrap();

        // The original 45 s deadline passes without a cancellation; the renewed one is still open.
        tokio::time::advance(Duration::from_secs(16)).await;
        assert!(
            fixture.frames.try_recv().is_err(),
            "renewal must survive the original first-window deadline"
        );

        // No further frames arrive: the renewed window expires at t = 75 s and retries.
        tokio::time::advance(Duration::from_secs(29)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        assert!(matches!(
            stream.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        fixture
            .agent
            .send(AgentInput::Finish(
                "sess-1".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// D6 stops at the delegation's provider completion: late child frames neither renew nor pause,
/// while an open delegation's frames do renew (the first assertion fails without the rule).
#[test]
fn subagent_frames_stop_renewing_once_the_parent_delegation_completed() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("sess-1");
        let mut stream = fixture
            .manager
            .prompt_session(prompt("sess-1"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        fixture
            .agent
            .send(AgentInput::Update(s1_parent_open()))
            .unwrap();
        stream.recv().await.unwrap().unwrap();
        tokio::time::advance(Duration::from_secs(10)).await;
        fixture
            .agent
            .send(AgentInput::RawFrame(s1_subagent_frame(json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "exploring" },
            }))))
            .unwrap();
        stream.recv().await.unwrap().unwrap();
        // Renewal moved the deadline past the original 45 s window.
        tokio::time::advance(Duration::from_secs(40)).await;
        assert!(fixture.frames.try_recv().is_err());

        // The provider completes the delegation; the window rearms from that completion.
        let completed: SessionNotification = serde_json::from_value(json!({
            "sessionId": "sess-1",
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call-1",
                "status": "completed",
                "content": [{ "type": "content", "content": { "type": "text", "text": "done" } }],
            },
        }))
        .unwrap();
        fixture.agent.send(AgentInput::Update(completed)).unwrap();
        stream.recv().await.unwrap().unwrap();
        tokio::time::advance(Duration::from_secs(10)).await;
        // A late child frame must not renew the closed delegation.
        fixture
            .agent
            .send(AgentInput::RawFrame(s1_subagent_frame(json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "late" },
            }))))
            .unwrap();
        stream.recv().await.unwrap().unwrap();
        tokio::time::advance(Duration::from_secs(35)).await;
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        assert!(matches!(
            stream.recv().await.unwrap().unwrap(),
            PromptSessionEvent::Retrying { retry: 1, .. }
        ));
        fixture
            .agent
            .send(AgentInput::Finish(
                "sess-1".to_string(),
                StopReason::EndTurn,
            ))
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

mod restored_sessions {
    mod command_catalog_tests;
    mod command_limit_tests;
    mod follower_retry_tests;
    mod follower_setup_tests;
    mod set_config_tests;
    mod setup_options_tests;
    use super::*;
    use pretty_assertions::assert_eq;
    /// Holds two or more sessions on one real routed ACP peer with a controllable provider.
    struct ReviewDrainFixture {
        _directory: TempDir,
        manager: crate::AgentRuntimeManager<TestHost>,
        supervisor: ConnectionSupervisor,
        plugin: PluginRuntime,
        frames: mpsc::UnboundedReceiver<Value>,
        routed: tokio::task::JoinHandle<()>,
        _state: watch::Sender<ConnectionState>,
        scheduler: ora_scheduler::Scheduler,
    }

    impl ReviewDrainFixture {
        /// Composes the production peer, generation router, and actors over controlled pipes.
        async fn new() -> Self {
            Self::new_with(review_drain_agent).await
        }

        /// Shares the real actor and router across tests with different provider response scripts.
        async fn new_with<Fut>(
            agent: impl FnOnce(
                DuplexStream,
                DuplexStream,
                mpsc::UnboundedReceiver<AgentInput>,
                mpsc::UnboundedSender<Value>,
                watch::Sender<bool>,
            ) -> Fut,
        ) -> Self
        where
            Fut: Future<Output = ()> + Send + 'static,
        {
            let directory = TempDir::new().unwrap();
            let entrypoint = directory.path().join("agent.ts");
            std::fs::write(&entrypoint, "").unwrap();
            let (stdin, agent_reader) = tokio::io::duplex(/*max_buf_size*/ 65536);
            let (agent_writer, stdout) = tokio::io::duplex(/*max_buf_size*/ 65536);
            let (exited, _) = watch::channel(false);
            let spawner = PipeSpawner(Mutex::new(Some(PipeProcess {
                stdin: Some(stdin),
                stdout: Some(stdout),
                exited: exited.clone(),
            })));
            let (_agent, input) = mpsc::unbounded_channel();
            let (sent, frames) = mpsc::unbounded_channel();
            tokio::spawn(agent(agent_reader, agent_writer, input, sent, exited));
            let (_level, level) = watch::channel(ora_logging::LogLevel::Info);
            let (plugin, mut notifications) = PluginRuntime::launch(
                &spawner,
                PluginRuntimeConfig {
                    plugin_id: "official/test.agent".to_string(),
                    deno_path: directory.path().join("unused-deno"),
                    entrypoint,
                    permissions: Vec::new(),
                    cwd: None,
                    environment: BTreeMap::new(),
                    ready_timeout: Duration::from_secs(30),
                    call_timeout: Duration::from_secs(30),
                    shutdown_timeout: Duration::from_secs(1),
                },
                NoHostRequests,
                PluginLogSetup {
                    root: directory.path().join("logs"),
                    directory: directory.path().join("logs").join("agent"),
                    host_session_id: "test".to_string(),
                    generation: 1,
                    level,
                },
            )
            .await
            .unwrap();
            let (messages, inbound) = mpsc::unbounded_channel();
            tokio::spawn(async move {
                while let Some(notification) = notifications.recv().await {
                    if messages.send(Ok(notification.params)).is_err() {
                        break;
                    }
                }
            });
            let (client, inbound) =
                AcpPeer::spawn(inbound, PluginAcpTransport::new(plugin.clone())).into_parts();
            let connection = RuntimeConnection {
                client: client.clone(),
                runtime: plugin.clone(),
                generation: 1,
                load_session_supported: true,
                http_mcp_supported: false,
                list_session_supported: false,
                close_session_supported: false,
                delete_session_supported: false,
            };
            let (state, receiver) = watch::channel(ConnectionState::Ready(connection));
            let (shutdown, mut shutdown_receiver) = mpsc::unbounded_channel();
            let routes = Arc::new(RouteRegistry::default());
            let active_generation = Arc::new(AtomicU64::new(1));
            let supervisor = ConnectionSupervisor {
                label: Arc::from("test"),
                state: receiver,
                active_generation: active_generation.clone(),
                routes: routes.clone(),
                shutdown,
            };
            let scheduler = ora_scheduler::Scheduler::new(chrono_tz::Asia::Shanghai);
            let runtime = test_runtime(
                directory.path(),
                Vec::new(),
                MemorySessionStore::default(),
                scheduler.clone(),
            );
            let host = runtime.manager.inner.connections.plugin_host.clone();
            let plugin_id = PluginId::parse("official/test.agent").unwrap();
            runtime
                .manager
                .inner
                .connections
                .supervisors
                .write()
                .unwrap()
                .insert(AgentRef::for_plugin(&plugin_id), supervisor.clone());
            let process_plugin = plugin.clone();
            let routed = tokio::spawn(async move {
                let mut process = startup::SharedProcess {
                    process: AgentProcess {
                        plugin_id,
                        runtime: process_plugin,
                        host,
                    },
                    client,
                    inbound,
                    load_session_supported: true,
                    http_mcp_supported: false,
                    list_session_supported: false,
                    close_session_supported: false,
                    delete_session_supported: false,
                };
                run_process_generation(&mut process, &routes, &mut shutdown_receiver).await;
                active_generation.store(0, Ordering::Release);
                routes
                    .fail_generation(1, runtime_unavailable_because("fake provider disconnected"));
            });
            Self {
                _directory: directory,
                manager: runtime.manager,
                supervisor,
                plugin,
                frames,
                routed,
                _state: state,
                scheduler,
            }
        }

        /// Persists a session bound to this agent, with no actor serving it.
        fn seed_session_on(&self, id: &str, agent_ref: &str) -> Session {
            self.manager
                .inner
                .store
                .create_session(Session::new(
                    SessionId::new(id),
                    WorkspaceId::new("workspace"),
                    AgentRef::parse(agent_ref).unwrap(),
                    id,
                    SessionStatus::Running,
                    SessionMcpSelection::Automatic,
                    AuditFields::new(
                        /*created_at*/ 0, /*updated_at*/ 0, /*is_deleted*/ false,
                    ),
                ))
                .unwrap()
        }

        /// Seeds an already attached session so the test measures prompt policy rather than setup.
        fn add_session(&self, id: &str) {
            self.add_session_with_title_acquisition(id, crate::TitleAcquisition::disabled());
        }

        /// Retains the title lifecycle established by setup so title admission can also be exercised.
        fn add_session_with_title_acquisition(
            &self,
            id: &str,
            title_acquisition: crate::TitleAcquisition,
        ) {
            self.add_session_with(
                id,
                title_acquisition,
                /*channel*/ None,
                LiveMcpState::Active(crate::SessionMcpRevision::default()),
            );
        }

        /// Seeds a session on a provider channel whose MCP set no longer is the desired one.
        fn add_session_owing_mcp_refresh(&self, id: &str) {
            let channel = self.supervisor.open_session_channel(id, id).unwrap();
            let stale =
                crate::SessionMcpRevision::for_generation(/*generation*/ 1, Vec::new());
            self.add_session_with(
                id,
                crate::TitleAcquisition::disabled(),
                Some(channel),
                LiveMcpState::Active(stale),
            );
        }

        /// Seeds one persisted session and the actor serving it.
        fn add_session_with(
            &self,
            id: &str,
            title_acquisition: crate::TitleAcquisition,
            channel: Option<crate::routing::SessionChannel>,
            live_mcp: LiveMcpState,
        ) {
            let session = self.seed_session_on(id, "official/test.agent");
            let recorder = self.manager.open_recorder(&session).unwrap().recorder;
            self.manager
                .insert_actor(
                    session,
                    ActorSetup {
                        session_mcp: NoSessionMcp::default(),
                        cwd: self._directory.path().to_path_buf(),
                        connection: self.supervisor.clone(),
                        channel,
                        recorder,
                        handoff: crate::HandoffDebt::Settled,
                        title_acquisition,
                        live_mcp,
                        config_options: Vec::new(),
                    },
                )
                .unwrap();
        }

        /// Observes a frame emitted by the actual plugin transport, without reading actor internals.
        async fn next_frame(&mut self) -> Value {
            self.frames.recv().await.expect("agent frame")
        }

        /// Ends every actor and the provider while retaining normal ownership cleanup.
        async fn shutdown(self) {
            for id in self
                .manager
                .inner
                .store
                .list_sessions()
                .unwrap()
                .into_iter()
                .map(|session| session.id.to_string())
            {
                let _ = self
                    .manager
                    .stop_session(ora_contracts::StopSessionRequest { session_id: id })
                    .await;
            }
            self.manager.inner.actors.write().unwrap().clear();
            self.plugin.shutdown_and_wait().await;
            self.routed.await.unwrap();
            self.scheduler.shutdown().await;
        }
    }

    /// A controllable protocol peer: successful load, output during cancellation settlement,
    /// failed inactivity resend, and (only if erroneously requested) successful replacement.
    async fn review_drain_agent(
        mut reader: DuplexStream,
        mut writer: DuplexStream,
        _commands: mpsc::UnboundedReceiver<AgentInput>,
        frames: mpsc::UnboundedSender<Value>,
        exited: watch::Sender<bool>,
    ) {
        write_message(
            &mut writer,
            &json!({"jsonrpc":"2.0","method":"ora/register",
        "params":{"methods":[],"emits":["agent/acp"]}}),
        )
        .await
        .unwrap();
        let mut prompt_count = 0;
        let mut held = Value::Null;
        while let Ok(Some(envelope)) = read_message(&mut reader).await {
            if envelope["method"] == "ora/shutdown" {
                break;
            }
            let frame = envelope["params"].clone();
            frames.send(frame.clone()).unwrap();
            let session = frame["params"]["sessionId"].as_str().unwrap_or("restored");
            let response = match frame["method"].as_str() {
                Some("session/load") => Some(json!({"jsonrpc":"2.0","id":frame["id"],"result":{}})),
                Some("session/new") => Some(json!({"jsonrpc":"2.0","id":frame["id"],
                "result":{"sessionId":"unwanted-replacement"}})),
                Some("session/prompt") => {
                    prompt_count += 1;
                    if prompt_count == 1 {
                        held = frame["id"].clone();
                        None
                    } else if prompt_count == 2 {
                        Some(json!({"jsonrpc":"2.0","id":frame["id"],"error":{
                        "code":-32000,"message":"request failed after cancellation output"}}))
                    } else {
                        Some(json!({"jsonrpc":"2.0","id":frame["id"],
                        "result":PromptResponse::new(StopReason::EndTurn)}))
                    }
                }
                Some("session/cancel") if !held.is_null() => {
                    let update = SessionNotification::new(
                        session.to_string(),
                        SessionUpdate::AgentMessageChunk(
                            agent_client_protocol_schema::v1::ContentChunk::new(
                                ContentBlock::Text(TextContent::new(
                                    "visible output before cancel fence",
                                )),
                            ),
                        ),
                    );
                    write_message(
                        &mut writer,
                        &json!({"jsonrpc":"2.0","method":"agent/acp",
                    "params":{"jsonrpc":"2.0","method":"session/update","params":update}}),
                    )
                    .await
                    .unwrap();
                    Some(json!({"jsonrpc":"2.0","id":std::mem::take(&mut held),
                    "result":PromptResponse::new(StopReason::Cancelled)}))
                }
                _ => None,
            };
            if let Some(response) = response {
                write_message(
                    &mut writer,
                    &json!({"jsonrpc":"2.0","method":"agent/acp",
                "params":response}),
                )
                .await
                .unwrap();
            }
        }
        exited.send_replace(true);
    }

    #[test]
    fn review_771_cancel_drain_output_must_prevent_stale_load_rebuild() {
        run_case(async {
            let mut fixture = ReviewDrainFixture::new().await;
            fixture.add_session("restored");
            let mut stream = fixture
                .manager
                .prompt_session(prompt("restored"))
                .await
                .unwrap();
            assert_eq!(fixture.next_frame().await["method"], "session/load");
            assert_eq!(fixture.next_frame().await["method"], "session/prompt");
            tokio::time::advance(Duration::from_secs(45)).await;
            let mut outputs = 0;
            let mut errors = Vec::new();
            let mut completed = 0;
            while let Some(event) = stream.recv().await {
                match event {
                    Ok(PromptSessionEvent::SessionUpdate {
                        update: SessionUpdate::AgentMessageChunk(_),
                        ..
                    }) => outputs += 1,
                    Ok(PromptSessionEvent::Completed { .. }) => completed += 1,
                    Err(error) => errors.push(format!("{error:?}")),
                    _ => {}
                }
            }
            drop(stream);
            let mut calls = Vec::new();
            while let Ok(frame) = fixture.frames.try_recv() {
                calls.push(frame);
            }
            let new_count = calls
                .iter()
                .filter(|frame| frame["method"] == "session/new")
                .count();
            let resent_count = calls
                .iter()
                .filter(|frame| frame["method"] == "session/prompt")
                .count();
            eprintln!(
                "review_771_drain_repro: outputs={outputs}; errors={errors:?}; completed={completed}; new={new_count}; resent={resent_count}; calls={calls:?}"
            );
            fixture.shutdown().await;
            assert_eq!(
                outputs, 1,
                "the cancellation drain really delivered assistant output"
            );
            assert_eq!(
                (new_count, resent_count, errors.len(), completed),
                (0, 1, 1, 0),
                "existing inactivity resend may run, but visible output must block the new stale-load rebuild; errors={errors:?}; calls={calls:?}"
            );
        });
    }
}

/// A workflow completion token is a host-only capability: the agent must receive it with every
/// turn, but the recorded user turn keeps only the user's words, so history, export, and the
/// transcript replayed into a rebuilt channel never carry a token.
#[test]
fn attempt_instruction_is_sent_but_not_recorded() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("instructed");
        let mut expected_records = Vec::new();
        // The first turn carries the assembled workflow prompt; the second has the follow-up
        // shape, a short user reply on the same session with a freshly issued token.
        for (task, token) in [
            ("workflow task instructions", "token-first-turn"),
            ("follow-up from the user", "token-follow-up"),
        ] {
            let instruction_text = format!(
                "<workflow_completion>\nattemptToken unchanged: {token}\n</workflow_completion>"
            );
            let sent_instruction = instruction_text.clone();
            let instruction: crate::AttemptInstruction =
                Box::new(move || ContentBlock::Text(TextContent::new(sent_instruction.clone())));
            let mut request = prompt("instructed");
            request.prompt = vec![ContentBlock::Text(TextContent::new(task))];
            let mut stream = fixture
                .manager
                .prompt_session_with_attempt_instruction(
                    request,
                    PromptInactivityPolicy::Wait,
                    Some(instruction),
                )
                .await
                .unwrap();
            let frame = fixture.next_frame().await;
            assert_eq!(frame["method"], "session/prompt");
            assert_eq!(
                frame["params"]["prompt"],
                json!([
                    {"type": "text", "text": task},
                    {"type": "text", "text": instruction_text},
                ])
            );
            fixture
                .agent
                .send(AgentInput::Finish(
                    "instructed".to_string(),
                    StopReason::EndTurn,
                ))
                .unwrap();
            assert_eq!(completion(&mut stream).await, StopReason::EndTurn);
            expected_records.push(ora_history::HistoryRecord::Update {
                update: Box::new(SessionUpdate::UserMessageChunk(
                    agent_client_protocol_schema::v1::ContentChunk::new(ContentBlock::Text(
                        TextContent::new(task),
                    )),
                )),
                tool_timing: None,
            });
            expected_records.push(ora_history::HistoryRecord::TurnEnded {
                stop_reason: StopReason::EndTurn,
            });
        }
        let records = ora_history::read_session_history(
            &fixture._directory.path().join("sessions"),
            "instructed",
        )
        .unwrap()
        .lines
        .into_iter()
        .map(|line| line.record)
        .collect::<Vec<_>>();
        assert_eq!(records, expected_records);
        fixture.shutdown().await;
    });
}
