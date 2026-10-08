//! Exercises prompt policy through the production admission, ACP peer, router, and session actor.

use super::*;
use crate::host::SessionStore;
use crate::session_setup::LiveMcpState;
use crate::suspend::AgentProcess;
use crate::test_host::{TestHost, test_runtime};
use crate::{ActorSetup, MemorySessionStore, NoSessionMcp, SessionEventStream};
use agent_client_protocol_schema::v1::{
    ContentBlock, PromptResponse, SessionNotification, SessionUpdate, StopReason, TextContent,
};
use ora_acp::AcpPeer;
use ora_contracts::{PromptSessionEvent, PromptSessionRequest};
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
    _state: watch::Sender<ConnectionState>,
    scheduler: ora_scheduler::Scheduler,
}

impl Fixture {
    /// Composes the production peer, generation router, and actors over controlled pipes.
    async fn new() -> Self {
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
            load_session_supported: false,
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
                load_session_supported: false,
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
            _state: state,
            scheduler,
        }
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
        let session = self
            .manager
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
            .unwrap();
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
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
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
                            (None, _) | (Some(_), CancelReply::Ignore) => None,
                        }
                    }
                    _ => { frames.send(frame).unwrap(); None }
                }
            }
            input = commands.recv() => match input {
                Some(AgentInput::Finish(session, reason)) => Some(json!({"jsonrpc":"2.0","id":pending.remove(&session).expect("pending prompt"),"result":PromptResponse::new(reason)})),
                Some(AgentInput::Update(update)) => Some(json!({"jsonrpc":"2.0","method":"session/update","params":update})),
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
    ora_logging::with_trace_logging(|| {
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
fn waiting_prompt_survives_silence_while_default_session_retries() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        fixture.add_session("default");
        let mut waiting = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
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

/// Explicit cancellation still obtains the actual provider fence after an arbitrarily long wait.
#[test]
fn waiting_prompt_remains_cancellable_and_reusable() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
            .await
            .unwrap();
        fixture.next_frame().await;
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
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
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

/// A real provider EOF terminates an opted-out prompt through the generation's failure control.
#[test]
fn provider_disconnect_ends_a_waiting_prompt() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("waiting");
        let mut stream = fixture
            .manager
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
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
            .prompt_session_with_inactivity_policy(prompt("waiting"), PromptInactivityPolicy::Wait)
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
            _state,
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
        drop((agent, _state, _directory));
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
