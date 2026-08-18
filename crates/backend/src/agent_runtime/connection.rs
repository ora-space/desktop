use super::plugin_agent::{
    self, AgentTransport, LaunchedPluginAgent, PluginAcpTransport, PluginAgentError,
    PluginAgentModel, PluginAgentSpec,
};
use super::routing::{RouteRegistry, SessionChannel, SessionEvent};
use super::{
    CANCELLATION_GRACE, CONTRACT_QUEUE_CAPACITY, INITIALIZE_TIMEOUT, map_acp_error,
    resolve_agent_cli_path, runtime_internal,
};
use crate::BackendError;
use crate::clock::SystemClock;
use agent_client_protocol_schema::ProtocolVersion;
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::{
    ClientCapabilities, ClientSessionCapabilities, Implementation, InitializeRequest,
    InitializeResponse, SessionConfigOptionsCapabilities,
};
use agent_client_protocol_schema::v1::{RequestPermissionOutcome, RequestPermissionResponse};
use ora_acp::{AcpClient, AcpInboundEvent, AcpMessages, AcpPeer, NdjsonTransport};
use ora_application::{Clock, SessionRepository};
use ora_contracts::PublicError;
use ora_db::{RepositoryPool, SqliteSessionRepository};
use ora_domain::{AgentCli, SessionStatus};
use ora_logging::{ora_error, ora_info, ora_warn};
use ora_plugin_runtime::PluginRuntime;
use ora_process::{
    ManagedProcess, ProcessSpawner, ProcessSpec, TokioManagedProcess, TokioProcessSpawner,
};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

/// Names the ACP transport every supervised agent connection speaks over.
///
/// `RuntimeConnection` is published through a `watch` channel, so the transport cannot stay
/// generic; naming it once keeps the rest of the runtime unaware of which transport is in use.
pub(super) type AgentAcpClient = AcpClient<AgentTransport>;

/// Selects how one supervised agent is started and who owns the process behind it.
///
/// The two variants differ only in startup and teardown. Once a connection is ready, every caller
/// above this module sees the same `RuntimeConnection` regardless of which variant produced it,
/// which is what lets plugin-provided and built-in agents coexist without branching elsewhere.
#[derive(Debug, Clone)]
pub(super) enum AgentSource {
    /// A CLI Ora launches itself and speaks NDJSON ACP to over its stdio pipes.
    Cli(AgentCli),
    /// A plugin package that owns its agent process and relays ACP frames to the host.
    Plugin(PluginAgentSpec),
}

impl AgentSource {
    /// Returns the persisted, namespaced identity of the agent this source provides.
    fn identifier(&self) -> &str {
        match self {
            Self::Cli(agent_cli) => agent_cli.database_value(),
            Self::Plugin(spec) => &spec.plugin_id,
        }
    }

    /// Returns the short name used for supervisor thread names and operator-facing messages.
    fn label(&self) -> &str {
        match self {
            Self::Cli(agent_cli) => agent_cli.executable_name(),
            Self::Plugin(spec) => &spec.plugin_id,
        }
    }
}

/// Exposes one initialized ACP connection without transferring child-process ownership.
#[derive(Clone)]
pub(super) struct RuntimeConnection {
    pub client: AgentAcpClient,
    pub generation: u64,
    pub load_session_supported: bool,
    /// Whether the agent advertises the bounded fallback used for first-title acquisition.
    pub list_session_supported: bool,
    pub close_session_supported: bool,
    /// Whether the agent advertises `session/delete`.
    ///
    /// Warm sessions Ora created but never handed to the user are removed with
    /// it so unused provider history does not accumulate; agents without it fall
    /// back to `session/close`, which only detaches.
    pub delete_session_supported: bool,
    /// Models this agent advertises outside any session, empty when it cannot advertise any.
    ///
    /// The list is read once per connection generation rather than on demand: it changes only
    /// when the provider restarts, and a reconnect already refreshes it.
    pub models: Arc<[PluginAgentModel]>,
}

#[derive(Clone)]
enum ConnectionState {
    Starting,
    Ready(RuntimeConnection),
    Unavailable,
}

/// Reports one CLI's live detection state without exposing its private connection handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectionStatus {
    Ready,
    Starting,
    Unavailable,
}

/// Keeps one supervisor generation's fixed dependencies together as the retry loop evolves.
struct SupervisorContext {
    source: AgentSource,
    pool: RepositoryPool,
    home_directory: PathBuf,
    clock: SystemClock,
    state: watch::Sender<ConnectionState>,
    active_generation: Arc<AtomicU64>,
    routes: Arc<RouteRegistry>,
    shutdown: mpsc::UnboundedReceiver<()>,
}

/// Gives session actors access to the current connection and central event router.
#[derive(Clone)]
pub(super) struct ConnectionSupervisor {
    label: Arc<str>,
    state: watch::Receiver<ConnectionState>,
    active_generation: Arc<AtomicU64>,
    routes: Arc<RouteRegistry>,
    shutdown: mpsc::UnboundedSender<()>,
}

/// Owns one independently supervised connection for every agent Ora can reach.
///
/// Agents are keyed by their persisted namespaced identity rather than by a closed enum, so a
/// plugin-provided agent is reachable through exactly the same lookup as a built-in CLI.
#[derive(Clone)]
pub(super) struct ConnectionSupervisors {
    supervisors: Arc<HashMap<String, ConnectionSupervisor>>,
}

impl ConnectionSupervisors {
    /// Starts every built-in CLI and every installed agent plugin eagerly.
    ///
    /// Availability stays independent per agent: one provider that is missing or crash-looping
    /// never delays or degrades the others, which is why each gets its own supervisor.
    pub fn start(
        agent_plugins: Vec<PluginAgentSpec>,
        pool: RepositoryPool,
        home_directory: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let sources = AgentCli::ALL
            .into_iter()
            .map(AgentSource::Cli)
            .chain(agent_plugins.into_iter().map(AgentSource::Plugin));
        let mut supervisors = HashMap::new();
        for source in sources {
            let identifier = source.identifier().to_string();
            // A plugin that shadows a built-in identity would silently replace it. Refusing keeps
            // the agent the user already had instead of handing it to an unvetted package.
            if supervisors.contains_key(&identifier) {
                ora_warn!(
                    agent = %identifier,
                    "ignoring an agent whose identity is already supervised"
                );
                continue;
            }
            supervisors.insert(
                identifier,
                ConnectionSupervisor::start(source, pool.clone(), home_directory.clone(), clock),
            );
        }
        Self {
            supervisors: Arc::new(supervisors),
        }
    }

    /// Selects the sole application-scoped connection for one persisted agent identity.
    ///
    /// A miss is a normal runtime state rather than data corruption: a session can outlive the
    /// plugin that provided its agent, and the caller reports that as an unavailable runtime.
    pub fn for_agent(&self, agent_cli: AgentCli) -> Result<ConnectionSupervisor, BackendError> {
        let identifier = agent_cli.database_value();
        self.supervisors.get(identifier).cloned().ok_or_else(|| {
            runtime_internal(
                "agent_runtime_unavailable",
                format!("{identifier} is not installed"),
            )
        })
    }
}

impl ConnectionSupervisor {
    /// Buffers otherwise-unrouted updates until `session/new` returns its provider id.
    pub fn begin_session_setup(&self) -> super::routing::SetupRegistration {
        self.routes.begin_session_setup()
    }

    /// Starts one application-scoped agent supervisor independently of the caller's runtime.
    pub(super) fn start(
        source: AgentSource,
        pool: RepositoryPool,
        home_directory: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let (state_sender, state) = watch::channel(ConnectionState::Unavailable);
        let (shutdown, shutdown_receiver) = mpsc::unbounded_channel();
        let active_generation = Arc::new(AtomicU64::new(0));
        let routes = Arc::new(RouteRegistry::default());
        let label: Arc<str> = Arc::from(source.label());
        let identifier = source.identifier().to_string();
        if let Err(error) = spawn_runtime_thread(
            &label,
            run_supervisor(SupervisorContext {
                source,
                pool,
                home_directory,
                clock,
                state: state_sender,
                active_generation: active_generation.clone(),
                routes: routes.clone(),
                shutdown: shutdown_receiver,
            }),
        ) {
            ora_warn!(
                agent = %identifier,
                error = %error,
                "agent supervisor thread could not start"
            );
        }
        Self {
            label,
            state,
            active_generation,
            routes,
            shutdown,
        }
    }

    /// Reports the live tri-state detection status without exposing the connection itself.
    pub fn status(&self) -> ConnectionStatus {
        match &*self.state.borrow() {
            ConnectionState::Ready(_) => ConnectionStatus::Ready,
            ConnectionState::Starting => ConnectionStatus::Starting,
            ConnectionState::Unavailable => ConnectionStatus::Unavailable,
        }
    }

    /// Returns the initialized shared connection or a stable degraded-runtime error.
    pub fn current(&self) -> Result<RuntimeConnection, BackendError> {
        match self.state.borrow().clone() {
            ConnectionState::Ready(connection) => Ok(connection),
            ConnectionState::Starting | ConnectionState::Unavailable => Err(runtime_internal(
                "agent_runtime_unavailable",
                format!("{label} runtime is unavailable", label = self.label),
            )),
        }
    }

    /// Registers a bounded ordered event route and independent failure controls for one session.
    pub fn open_session_channel(
        &self,
        agent_session_id: &str,
        ora_session_id: &str,
    ) -> Result<SessionChannel, BackendError> {
        let connection = self.current()?;
        if self.active_generation.load(Ordering::Acquire) != connection.generation {
            return Err(runtime_internal(
                "agent_runtime_unavailable",
                format!("{label} runtime is recovering", label = self.label),
            ));
        }
        let (events_sender, events) = mpsc::channel(CONTRACT_QUEUE_CAPACITY);
        let (controls_sender, controls) = mpsc::unbounded_channel();
        let trace_registration = connection
            .client
            .register_session_trace(agent_session_id, ora_session_id);
        let registration = self.routes.register(
            agent_session_id,
            connection.generation,
            events_sender,
            controls_sender,
        );
        if self.active_generation.load(Ordering::Acquire) != connection.generation {
            drop(registration);
            return Err(runtime_internal(
                "agent_runtime_unavailable",
                format!("{label} runtime is recovering", label = self.label),
            ));
        }
        Ok(SessionChannel {
            connection,
            events,
            pending_updates: std::collections::VecDeque::new(),
            controls,
            _trace_registration: trace_registration,
            _registration: registration,
        })
    }
}

/// Runs the supervisor on a dedicated runtime because Desktop bootstrap is synchronous.
fn spawn_runtime_thread<Supervisor>(label: &str, supervisor: Supervisor) -> std::io::Result<()>
where
    Supervisor: Future<Output = ()> + Send + 'static,
{
    let thread_label = label.to_string();
    std::thread::Builder::new()
        .name(format!("ora-{thread_label}-supervisor"))
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    ora_error!(
                        agent = %thread_label,
                        error = %error,
                        "agent supervisor runtime could not start"
                    );
                    return;
                }
            };
            runtime.block_on(supervisor);
        })
        .map(|_| ())
}

impl Drop for ConnectionSupervisor {
    fn drop(&mut self) {
        if self.shutdown.strong_count() == 1 {
            let _ = self.shutdown.send(());
        }
    }
}

/// Owns whatever process backs one connection generation for that generation's whole lifetime.
///
/// A built-in CLI is Ora's own child. A plugin instead owns the agent it fronts, so the host holds
/// the plugin runtime: dropping it sends `ora/shutdown` and reaps the plugin's process tree, which
/// is the host's standing guarantee regardless of how well the plugin cleans up after itself.
enum AgentProcess {
    // Boxed because a managed child process is far larger than a plugin handle, and every
    // connection would otherwise carry the bigger variant's footprint.
    Cli(Box<TokioManagedProcess>),
    Plugin(PluginRuntime),
}

impl AgentProcess {
    /// Reaps a failed generation before its replacement so two generations cannot overlap.
    async fn terminate_and_reap(&self, plugin_id: &str) {
        match self {
            Self::Cli(child) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            Self::Plugin(runtime) => {
                // Stopping the agent is the plugin's chance to reap the CLI it owns; ending the
                // plugin itself cannot wait for the last transport clone to be dropped, or a
                // surviving session actor would keep a failed plugin running.
                plugin_agent::stop_agent(runtime, plugin_id).await;
                runtime.shutdown();
            }
        }
    }

    /// Bounds application shutdown even when the operating system does not promptly reap a child.
    async fn stop_with_grace(&self, plugin_id: &str) {
        let _ = timeout(CANCELLATION_GRACE, self.terminate_and_reap(plugin_id)).await;
    }
}

/// Separates a startup failure worth retrying from one that can never succeed.
///
/// Almost every failure is retryable: a CLI can be installed later, a crashed provider can come
/// back. A provider that does not implement the contract this host requires is different — it will
/// fail identically forever, so retrying only produces a warning every backoff interval and never
/// a working agent.
enum StartFailure {
    Retryable(BackendError),
    Terminal(BackendError),
}

impl From<BackendError> for StartFailure {
    fn from(error: BackendError) -> Self {
        Self::Retryable(error)
    }
}

/// Holds everything one agent source produces before the ACP handshake runs.
struct StartedAgent {
    process: AgentProcess,
    transport: AgentTransport,
    messages: AcpMessages,
    models: Vec<PluginAgentModel>,
}

struct SharedProcess {
    process: AgentProcess,
    client: AgentAcpClient,
    models: Arc<[PluginAgentModel]>,
    inbound: mpsc::UnboundedReceiver<AcpInboundEvent>,
    load_session_supported: bool,
    list_session_supported: bool,
    close_session_supported: bool,
    delete_session_supported: bool,
}

/// Supervises one process generation at a time and retries only after it is fully reaped.
async fn run_supervisor(context: SupervisorContext) {
    let SupervisorContext {
        source,
        pool,
        home_directory,
        clock,
        state,
        active_generation,
        routes,
        mut shutdown,
    } = context;
    let identifier = source.identifier();
    let mut retry_delay = INITIAL_RETRY_DELAY;
    let mut generation = 0_u64;
    loop {
        let _ = state.send(ConnectionState::Starting);
        match spawn_initialized_process(&source, &home_directory).await {
            Ok(mut process) => {
                generation += 1;
                retry_delay = INITIAL_RETRY_DELAY;
                active_generation.store(generation, Ordering::Release);
                let connection = RuntimeConnection {
                    client: process.client.clone(),
                    models: process.models.clone(),
                    generation,
                    load_session_supported: process.load_session_supported,
                    list_session_supported: process.list_session_supported,
                    close_session_supported: process.close_session_supported,
                    delete_session_supported: process.delete_session_supported,
                };
                let _ = state.send(ConnectionState::Ready(connection));
                ora_info!(agent = identifier, generation, "agent runtime is ready");
                let shutting_down =
                    run_process_generation(&mut process, &routes, &mut shutdown).await;
                active_generation.store(0, Ordering::Release);
                let _ = state.send(ConnectionState::Unavailable);
                let error =
                    runtime_internal("agent_runtime_unavailable", "agent connection was lost");
                routes.fail_generation(generation, error);
                mark_running_sessions_stopped(&pool, clock, identifier);
                if shutting_down {
                    process.process.stop_with_grace(identifier).await;
                    return;
                }
                process.process.terminate_and_reap(identifier).await;
                ora_warn!(
                    agent = identifier,
                    generation,
                    "agent connection failed; scheduling restart"
                );
            }
            Err(StartFailure::Terminal(error)) => {
                let _ = state.send(ConnectionState::Unavailable);
                ora_warn!(
                    agent = identifier,
                    error = %error,
                    "agent cannot serve this host; giving up on it for this process"
                );
                return;
            }
            Err(StartFailure::Retryable(error)) => {
                let _ = state.send(ConnectionState::Unavailable);
                // An agent that is simply not installed is an expected local configuration, and
                // the supervisor keeps retrying it for the whole process lifetime. Logging it
                // would flood the runtime log with one line per retry while
                // `ConnectionState::Unavailable` already carries that fact to the UI, so only
                // genuine startup failures are logged.
                if !matches!(error.public_error(), PublicError::AgentCliNotFound(_)) {
                    ora_warn!(
                        agent = identifier,
                        error = %error,
                        "agent startup failed; scheduling retry"
                    );
                }
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(retry_delay) => {}
            _ = shutdown.recv() => return,
        }
        retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
    }
}

/// Drains and demultiplexes one live connection until shutdown or a transport-level failure.
async fn run_process_generation(
    process: &mut SharedProcess,
    routes: &RouteRegistry,
    shutdown: &mut mpsc::UnboundedReceiver<()>,
) -> bool {
    loop {
        tokio::select! {
            inbound = process.inbound.recv() => {
                match inbound {
                    Some(AcpInboundEvent::SessionUpdate(update)) => {
                        let _ = routes.route_event(SessionEvent::Update(update));
                    }
                    Some(AcpInboundEvent::PermissionRequest(permission)) => {
                        if let Err(orphan) = routes.route_event(SessionEvent::Permission(permission)) {
                            match *orphan {
                                SessionEvent::Permission(orphan) => {
                                    let _ = process.client.respond(
                                        &orphan.request_id,
                                        &RequestPermissionResponse::new(
                                            RequestPermissionOutcome::Cancelled,
                                        ),
                                    ).await;
                                }
                                SessionEvent::Update(_) | SessionEvent::Response(_) => {}
                            }
                        }
                    }
                    Some(AcpInboundEvent::SessionResponse(response)) => {
                        let _ = routes.route_event(SessionEvent::Response(response));
                    }
                    Some(AcpInboundEvent::Fatal(error)) => {
                        ora_warn!(
                            error = %error,
                            "agent ACP connection failed"
                        );
                        return false;
                    }
                    None => return false,
                }
            }
            _ = shutdown.recv() => return true,
        }
    }
}

/// Starts one agent in the neutral home directory and completes the ACP handshake.
///
/// Both sources converge here on purpose: whichever way the agent was started, the connection is
/// only reported ready once ACP `initialize` has returned its capabilities, so no caller can send
/// a session request to a transport that is not yet carrying a live agent.
async fn spawn_initialized_process(
    source: &AgentSource,
    home_directory: &Path,
) -> Result<SharedProcess, StartFailure> {
    let StartedAgent {
        process,
        transport,
        messages,
        models,
    } = match source {
        AgentSource::Cli(agent_cli) => spawn_cli_connection(*agent_cli, home_directory).await?,
        AgentSource::Plugin(spec) => spawn_plugin_connection(spec, home_directory).await?,
    };
    let peer = AcpPeer::spawn(messages, transport);
    // Config options are only sent by agents that see the client advertise them,
    // so the model selector depends on this declaration. Boolean options stay
    // undeclared because Ora renders only select-style options today; claiming
    // support would invite payloads the client silently drops.
    let initialize = InitializeRequest::new(ProtocolVersion::V1)
        .client_capabilities(
            ClientCapabilities::new().session(
                ClientSessionCapabilities::new()
                    .config_options(SessionConfigOptionsCapabilities::new()),
            ),
        )
        .client_info(Implementation::new("ora", env!("CARGO_PKG_VERSION")));
    let response = match timeout(
        INITIALIZE_TIMEOUT,
        peer.client
            .request::<_, InitializeResponse>(AGENT_METHOD_NAMES.initialize, &initialize),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            process.terminate_and_reap(source.identifier()).await;
            return Err(StartFailure::Retryable(map_acp_error(error)));
        }
        Err(_) => {
            process.terminate_and_reap(source.identifier()).await;
            return Err(StartFailure::Retryable(runtime_internal(
                "agent_initialize_timeout",
                "agent initialization timed out",
            )));
        }
    };
    let (client, inbound) = peer.into_parts();
    Ok(SharedProcess {
        process,
        client,
        models: models.into(),
        inbound,
        load_session_supported: response.agent_capabilities.load_session,
        list_session_supported: response
            .agent_capabilities
            .session_capabilities
            .list
            .is_some(),
        close_session_supported: response
            .agent_capabilities
            .session_capabilities
            .close
            .is_some(),
        delete_session_supported: response
            .agent_capabilities
            .session_capabilities
            .delete
            .is_some(),
    })
}

/// Launches one built-in CLI and wires NDJSON ACP over its stdio pipes.
async fn spawn_cli_connection(
    agent_cli: AgentCli,
    home_directory: &Path,
) -> Result<StartedAgent, StartFailure> {
    let executable = resolve_agent_cli_path(
        agent_cli,
        std::env::var_os("PATH").as_deref(),
        home_directory,
    )?;
    let mut child = TokioProcessSpawner::new()
        .spawn(
            ProcessSpec::new(executable)
                .args(agent_cli.launch_arguments())
                .cwd(home_directory),
        )
        .map_err(|source| BackendError::internal("failed to start agent CLI", source))?;
    let stdio = child.take_stdin().zip(child.take_stdout());
    let Some((stdin, stdout)) = stdio else {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(StartFailure::Retryable(runtime_internal(
            "agent_start_failed",
            "agent CLI stdio is unavailable",
        )));
    };
    if let Some(stderr) = child.take_stderr() {
        tokio::spawn(super::drain_stderr(stderr));
    }
    let (transport, messages) = NdjsonTransport::spawn(stdout, stdin);
    Ok(StartedAgent {
        process: AgentProcess::Cli(Box::new(child)),
        transport: AgentTransport::Stdio(transport),
        messages,
        // A built-in CLI has no pre-session model list; its models arrive as ACP session config
        // options once a session exists.
        models: Vec::new(),
    })
}

/// Launches one agent plugin and wires ACP over its notification channel.
async fn spawn_plugin_connection(
    spec: &PluginAgentSpec,
    home_directory: &Path,
) -> Result<StartedAgent, StartFailure> {
    let LaunchedPluginAgent { runtime, messages } =
        plugin_agent::launch(spec, home_directory, env!("CARGO_PKG_VERSION"))
            .await
            .map_err(plugin_start_error)?;
    let models = plugin_agent::list_models(&runtime)
        .await
        .map_err(plugin_start_error)?;
    let transport = AgentTransport::Plugin(PluginAcpTransport::new(runtime.clone()));
    Ok(StartedAgent {
        process: AgentProcess::Plugin(runtime),
        transport,
        messages,
        models,
    })
}

/// Maps a plugin startup failure onto the same public shape a missing CLI already produces.
///
/// A plugin whose agent is not installed must be indistinguishable from a CLI that is not
/// installed, because the supervisor treats that case as an expected local configuration and
/// retries it without logging.
fn plugin_start_error(error: PluginAgentError) -> StartFailure {
    match error {
        PluginAgentError::AgentNotInstalled => StartFailure::Retryable(runtime_internal(
            "agent_cli_not_found",
            "the agent behind this plugin is not installed",
        )),
        PluginAgentError::ContractIncomplete(detail) => {
            StartFailure::Terminal(runtime_internal("agent_start_failed", detail))
        }
        PluginAgentError::Failed(detail) => {
            StartFailure::Retryable(runtime_internal("agent_start_failed", detail))
        }
    }
}

/// Persists one agent's connection loss without stopping sessions owned by healthy agents.
fn mark_running_sessions_stopped(pool: &RepositoryPool, clock: SystemClock, identifier: &str) {
    let repository = SqliteSessionRepository::new(pool.clone());
    let Ok(sessions) = repository.list_sessions() else {
        return;
    };
    for session in sessions {
        if session.agent_cli.database_value() == identifier
            && session.status == SessionStatus::Running
        {
            let _ = repository.update_session_status(
                &session.id,
                SessionStatus::Stopped,
                clock.now_timestamp_millis(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginAgentError, StartFailure, plugin_start_error, spawn_runtime_thread};
    use ora_contracts::PublicError;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    /// Verifies synchronous bootstrap can launch async supervision without an ambient runtime.
    #[test]
    fn starts_a_dedicated_runtime_thread() {
        let (sender, receiver) = std::sync::mpsc::channel();

        spawn_runtime_thread("opencode", async move {
            sender.send("ready").expect("send runtime signal");
        })
        .expect("start runtime thread");

        assert_eq!(receiver.recv_timeout(Duration::from_secs(1)), Ok("ready"));
    }

    /// Verifies a missing agent stays retryable and reports the same cause as a missing CLI.
    #[test]
    fn treats_a_missing_plugin_agent_like_a_missing_cli() {
        let failure = plugin_start_error(PluginAgentError::AgentNotInstalled);

        let StartFailure::Retryable(error) = failure else {
            panic!("a missing agent must stay retryable");
        };
        assert!(matches!(
            error.public_error(),
            PublicError::AgentCliNotFound(_)
        ));
    }

    /// Verifies a plugin that cannot serve the contract is abandoned instead of retried forever.
    #[test]
    fn gives_up_on_a_plugin_that_cannot_serve_the_contract() {
        let failure =
            plugin_start_error(PluginAgentError::ContractIncomplete("missing".to_string()));

        assert!(matches!(failure, StartFailure::Terminal(_)));
    }

    /// Verifies an ordinary startup failure is retried, because the agent may recover.
    #[test]
    fn retries_an_ordinary_plugin_startup_failure() {
        let failure = plugin_start_error(PluginAgentError::Failed("spawn refused".to_string()));

        assert!(matches!(failure, StartFailure::Retryable(_)));
    }
}
