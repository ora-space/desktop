use super::plugin_agent::{
    self, AgentTransport, LaunchedPluginAgent, PluginAcpTransport, PluginAgentError,
    PluginAgentModel,
};
use super::restart_circuit::{RestartCircuit, RestartDecision};
use super::routing::{RouteRegistry, SessionChannel, SessionEvent};
use super::{
    CANCELLATION_GRACE, CONTRACT_QUEUE_CAPACITY, INITIALIZE_TIMEOUT, map_acp_error,
    resolve_agent_cli_path, runtime_internal,
};
use crate::BackendError;
use crate::clock::SystemClock;
use crate::plugin::PluginApi;
use agent_client_protocol_schema::ProtocolVersion;
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::{
    ClientCapabilities, ClientSessionCapabilities, Implementation, InitializeRequest,
    InitializeResponse, SessionConfigOptionsCapabilities,
};
use agent_client_protocol_schema::v1::{RequestPermissionOutcome, RequestPermissionResponse};
use ora_acp::{AcpClient, AcpInboundEvent, AcpMessages, AcpPeer, NdjsonTransport};
use ora_application::{Clock, SessionRepository};
use ora_contracts::{
    InstalledPluginContribution, ListInstalledPluginsRequest, PublicError, StopPluginRequest,
};
use ora_db::{RepositoryPool, SqliteSessionRepository};
use ora_domain::{AgentCli, AgentRef, PluginId, SessionStatus};
use ora_logging::{ora_error, ora_info, ora_warn};
use ora_plugin_lifecycle::ConnectionError;
use ora_plugin_runtime::PluginRuntime;
use ora_process::{
    ManagedProcess, ProcessSpawner, ProcessSpec, TokioManagedProcess, TokioProcessSpawner,
};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};
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
    ///
    /// A package carries two distinct identities and both are needed here. `plugin_id` is the
    /// package address — `namespace/name` — that the plugin lifecycle owns the process under.
    /// `package_name` is the agent identity the package supplies, which is what sessions persist
    /// as their `agent_ref`. Supervising a plugin under its package address instead would leave
    /// every stored binding pointing at an agent no lookup can reach.
    Plugin {
        plugin_id: PluginId,
        package_name: String,
    },
}

impl AgentSource {
    /// Returns the persisted, namespaced identity of the agent this source provides.
    fn agent_ref(&self) -> Result<AgentRef, BackendError> {
        match self {
            Self::Cli(agent_cli) => Ok(agent_cli.agent_ref()),
            // A package name reaches here already validated by discovery, but parsing keeps one
            // construction path for the value object rather than a second, unchecked one.
            Self::Plugin { package_name, .. } => AgentRef::parse(package_name)
                .map_err(|error| runtime_internal("agent_start_failed", error.to_string())),
        }
    }

    /// Returns the short name used for supervisor thread names and operator-facing messages.
    fn label(&self) -> &str {
        match self {
            Self::Cli(agent_cli) => agent_cli.executable_name(),
            Self::Plugin { plugin_id, .. } => plugin_id.name(),
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
    Failing,
}

/// Reports one CLI's live detection state without exposing its private connection handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectionStatus {
    Ready,
    Starting,
    Unavailable,
    Failing,
}

/// Keeps one supervisor generation's fixed dependencies together as the retry loop evolves.
struct SupervisorContext {
    agent_ref: AgentRef,
    source: AgentSource,
    /// Starts and stops the processes behind plugin-provided agents.
    plugin_host: Arc<PluginApi>,
    pool: RepositoryPool,
    home_directory: PathBuf,
    clock: SystemClock,
    state: watch::Sender<ConnectionState>,
    active_generation: Arc<AtomicU64>,
    routes: Arc<RouteRegistry>,
    shutdown: mpsc::UnboundedReceiver<()>,
    wake: mpsc::UnboundedReceiver<()>,
}

/// Gives session actors access to the current connection and central event router.
#[derive(Clone)]
pub(super) struct ConnectionSupervisor {
    label: Arc<str>,
    state: watch::Receiver<ConnectionState>,
    active_generation: Arc<AtomicU64>,
    routes: Arc<RouteRegistry>,
    shutdown: mpsc::UnboundedSender<()>,
    /// Cuts the current retry backoff short when something made this agent usable.
    wake: mpsc::UnboundedSender<()>,
}

/// Owns one independently supervised connection for every agent Ora can reach.
///
/// Agents are keyed by their persisted namespaced identity rather than by a closed enum, so a
/// plugin-provided agent is reachable through exactly the same lookup as a built-in CLI.
///
/// The set is mutable because installing a plugin adds an agent while Ora is running. It is held
/// behind a lock rather than rebuilt, so every existing clone — the warm pool, live session
/// actors — observes an install or uninstall without being handed a new value.
#[derive(Clone)]
pub(super) struct ConnectionSupervisors {
    supervisors: Arc<RwLock<BTreeMap<AgentRef, ConnectionSupervisor>>>,
    /// Retained so a package installed after startup can be supervised without restarting Ora.
    plugin_host: Arc<PluginApi>,
    pool: RepositoryPool,
    home_directory: PathBuf,
    clock: SystemClock,
}

impl ConnectionSupervisors {
    /// Starts every built-in CLI and every installed agent plugin eagerly.
    ///
    /// Availability stays independent per agent: one provider that is missing or crash-looping
    /// never delays or degrades the others, which is why each gets its own supervisor.
    ///
    /// Every installed plugin is supervised regardless of whether it is enabled: eligibility is
    /// the lifecycle's answer to give, and a disabled plugin simply keeps failing to attach until
    /// the user enables it.
    pub fn start(
        plugin_host: Arc<PluginApi>,
        pool: RepositoryPool,
        home_directory: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let supervisors = Self {
            supervisors: Arc::new(RwLock::new(BTreeMap::new())),
            plugin_host,
            pool,
            home_directory,
            clock,
        };
        supervisors.sync_plugin_agents();
        supervisors
    }

    /// Reconciles the supervised agents with the packages currently installed.
    ///
    /// Installing a plugin has to make its agent reachable in the running process: the alternative
    /// is a chat that reports the agent as not installed until Ora is restarted, while the settings
    /// surface already lists the plugin. Uninstalling one drops its supervisor for the same reason.
    ///
    /// Built-in CLIs are always part of the desired set, so they are established on the first call
    /// and never removed by a later one.
    pub fn sync_plugin_agents(&self) {
        // Only agent-kind packages supply an agent; ui packages contribute surfaces and are never
        // supervised here. Ids in the snapshot are canonical, so an unparsable one cannot occur
        // and is simply skipped rather than aborting the reconciliation.
        let agent_plugins = self
            .plugin_host
            .list(ListInstalledPluginsRequest {})
            .plugins
            .into_iter()
            .filter(|plugin| {
                matches!(
                    plugin.contribution,
                    InstalledPluginContribution::Agent { .. }
                )
            })
            .filter_map(|plugin| {
                PluginId::parse(&plugin.id)
                    .ok()
                    .map(|plugin_id| AgentSource::Plugin {
                        plugin_id,
                        package_name: plugin.name,
                    })
            });
        let desired = resolve_supervised_agents(
            AgentCli::ALL
                .into_iter()
                .map(AgentSource::Cli)
                .chain(agent_plugins),
        );

        let mut supervisors = self
            .supervisors
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let desired_refs = desired
            .iter()
            .map(|(agent_ref, _source)| agent_ref.clone())
            .collect::<BTreeSet<_>>();
        // Dropping the map's handle only signals shutdown once every session actor holding a clone
        // has released it, so an uninstall never severs a conversation that is still open.
        supervisors.retain(|agent_ref, _supervisor| desired_refs.contains(agent_ref));
        for (agent_ref, source) in desired {
            if supervisors.contains_key(&agent_ref) {
                continue;
            }
            let supervisor = ConnectionSupervisor::start(
                agent_ref.clone(),
                source,
                self.plugin_host.clone(),
                self.pool.clone(),
                self.home_directory.clone(),
                self.clock,
            );
            supervisors.insert(agent_ref, supervisor);
        }
    }

    /// Retries one agent at once rather than at the end of its current backoff.
    ///
    /// Enabling a plugin is the moment its agent becomes usable, and the supervisor that has been
    /// refusing to attach may be most of a backoff interval away from noticing. An identity this
    /// host does not supervise is ignored: nothing is waiting on it.
    pub fn wake_agent(&self, agent_ref: &AgentRef) {
        if let Some(supervisor) = self
            .supervisors
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(agent_ref)
        {
            supervisor.wake();
        }
    }

    /// Selects the sole application-scoped connection for one persisted agent identity.
    ///
    /// A miss is a normal runtime state rather than data corruption: a session can outlive the
    /// plugin that provided its agent, and the caller reports that as an unavailable runtime.
    pub fn for_agent(&self, agent_ref: &AgentRef) -> Result<ConnectionSupervisor, BackendError> {
        self.supervisors
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(agent_ref)
            .cloned()
            .ok_or_else(|| {
                runtime_internal(
                    "agent_runtime_unavailable",
                    format!("{agent_ref} is not installed"),
                )
            })
    }

    /// Reports every supervised agent with its live status, in stable identity order.
    ///
    /// Enumerating what is actually supervised is what lets a plugin-provided agent appear in the
    /// picker: the set is no longer knowable at build time.
    pub fn statuses(&self) -> Vec<(AgentRef, ConnectionStatus)> {
        self.supervisors
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(agent_ref, supervisor)| (agent_ref.clone(), supervisor.status()))
            .collect()
    }
}

impl ConnectionSupervisor {
    /// Buffers otherwise-unrouted updates until `session/new` returns its provider id.
    pub fn begin_session_setup(&self) -> super::routing::SetupRegistration {
        self.routes.begin_session_setup()
    }

    /// Starts one application-scoped agent supervisor independently of the caller's runtime.
    pub(super) fn start(
        agent_ref: AgentRef,
        source: AgentSource,
        plugin_host: Arc<PluginApi>,
        pool: RepositoryPool,
        home_directory: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let (state_sender, state) = watch::channel(ConnectionState::Unavailable);
        let (shutdown, shutdown_receiver) = mpsc::unbounded_channel();
        let (wake, wake_receiver) = mpsc::unbounded_channel();
        let active_generation = Arc::new(AtomicU64::new(0));
        let routes = Arc::new(RouteRegistry::default());
        let label: Arc<str> = Arc::from(source.label());
        let identifier = agent_ref.to_string();
        if let Err(error) = spawn_runtime_thread(
            &label,
            run_supervisor(SupervisorContext {
                agent_ref,
                source,
                plugin_host,
                pool,
                home_directory,
                clock,
                state: state_sender,
                active_generation: active_generation.clone(),
                routes: routes.clone(),
                shutdown: shutdown_receiver,
                wake: wake_receiver,
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
            wake,
        }
    }

    /// Asks the retry loop to attempt this agent now instead of waiting out its backoff.
    ///
    /// A supervisor that has already given up — a crash loop, or a provider that cannot serve this
    /// host's contract — has ended its loop, so this is deliberately best effort.
    pub(super) fn wake(&self) {
        let _ = self.wake.send(());
    }

    /// Reports the live tri-state detection status without exposing the connection itself.
    pub fn status(&self) -> ConnectionStatus {
        match &*self.state.borrow() {
            ConnectionState::Ready(_) => ConnectionStatus::Ready,
            ConnectionState::Starting => ConnectionStatus::Starting,
            ConnectionState::Unavailable => ConnectionStatus::Unavailable,
            ConnectionState::Failing => ConnectionStatus::Failing,
        }
    }

    /// Returns the initialized shared connection or a stable degraded-runtime error.
    pub fn current(&self) -> Result<RuntimeConnection, BackendError> {
        match self.state.borrow().clone() {
            ConnectionState::Ready(connection) => Ok(connection),
            ConnectionState::Starting | ConnectionState::Unavailable | ConnectionState::Failing => {
                Err(runtime_internal(
                    "agent_runtime_unavailable",
                    format!("{label} runtime is unavailable", label = self.label),
                ))
            }
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

/// Decides which agent identity each source supervises, in the order the sources were offered.
///
/// Built-in CLIs are offered first, so a plugin that claims an identity already taken is dropped
/// rather than allowed to replace it: silently handing a user's existing agent to an unvetted
/// package is worse than ignoring the package. A source whose identity is unusable is dropped for
/// the same reason — there would be no way to address it.
fn resolve_supervised_agents(
    sources: impl Iterator<Item = AgentSource>,
) -> Vec<(AgentRef, AgentSource)> {
    let mut claimed = BTreeSet::new();
    let mut resolved = Vec::new();
    for source in sources {
        let Ok(agent_ref) = source.agent_ref() else {
            ora_warn!(
                agent = source.label(),
                "ignoring an agent whose identity is not a usable reference"
            );
            continue;
        };
        if !claimed.insert(agent_ref.clone()) {
            ora_warn!(
                agent = %agent_ref,
                "ignoring an agent whose identity is already supervised"
            );
            continue;
        }
        resolved.push((agent_ref, source));
    }
    resolved
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

/// Ends whatever process backs one connection generation once that generation is over.
///
/// A built-in CLI is Ora's own child, so the supervisor reaps it directly. A plugin process
/// belongs to the plugin lifecycle instead: this connection only borrowed its ACP stream, so
/// ending the generation means telling the lifecycle to stop it, which keeps the runtime state
/// the settings surface reports honest and leaves the next attach to start a fresh process.
enum AgentProcess {
    // Boxed because a managed child process is far larger than a plugin handle, and every
    // connection would otherwise carry the bigger variant's footprint.
    Cli(Box<TokioManagedProcess>),
    Plugin {
        plugin_id: PluginId,
        runtime: PluginRuntime,
        host: Arc<PluginApi>,
    },
}

impl AgentProcess {
    /// Reaps a failed generation before its replacement so two generations cannot overlap.
    async fn terminate_and_reap(&self) {
        match self {
            Self::Cli(child) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            Self::Plugin {
                plugin_id,
                runtime,
                host,
            } => {
                // Stopping the agent is the plugin's chance to reap the CLI it owns before the
                // lifecycle ends the plugin process itself.
                plugin_agent::stop_agent(runtime, &plugin_id.canonical()).await;
                stop_plugin_runtime(host, plugin_id).await;
            }
        }
    }

    /// Bounds application shutdown even when the operating system does not promptly reap a child.
    async fn stop_with_grace(&self) {
        let _ = timeout(CANCELLATION_GRACE, self.terminate_and_reap()).await;
    }
}

/// Asks the lifecycle to end one plugin process after its agent generation failed or shut down.
///
/// A stop that itself fails is logged rather than propagated: the caller is already tearing a
/// generation down, and the next attach restarts the plugin regardless of what this left behind.
async fn stop_plugin_runtime(host: &PluginApi, plugin_id: &PluginId) {
    if let Err(error) = host
        .stop(StopPluginRequest {
            plugin_id: plugin_id.to_string(),
        })
        .await
    {
        ora_warn!(
            plugin_id = %plugin_id,
            error = %error,
            "plugin runtime could not be stopped after its agent generation ended"
        );
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
        agent_ref,
        source,
        plugin_host,
        pool,
        home_directory,
        clock,
        state,
        active_generation,
        routes,
        mut shutdown,
        mut wake,
    } = context;
    let identifier = agent_ref.as_str();
    let mut retry_delay = INITIAL_RETRY_DELAY;
    let mut generation = 0_u64;
    let mut restart_circuit = RestartCircuit::default();
    loop {
        let _ = state.send(ConnectionState::Starting);
        match spawn_initialized_process(&source, &plugin_host, &home_directory).await {
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
                mark_running_sessions_stopped(&pool, clock, &agent_ref);
                if shutting_down {
                    process.process.stop_with_grace().await;
                    return;
                }
                process.process.terminate_and_reap().await;
                if restart_circuit.record_failure(Instant::now()) == RestartDecision::Stop {
                    let _ = state.send(ConnectionState::Failing);
                    ora_warn!(
                        agent = identifier,
                        generation,
                        "agent entered a crash loop; automatic restarts are disabled"
                    );
                    return;
                }
                ora_warn!(
                    agent = identifier,
                    generation,
                    "agent connection failed; scheduling restart"
                );
            }
            Err(StartFailure::Terminal(error)) => {
                let _ = state.send(ConnectionState::Failing);
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
                    if restart_circuit.record_failure(Instant::now()) == RestartDecision::Stop {
                        let _ = state.send(ConnectionState::Failing);
                        ora_warn!(
                            agent = identifier,
                            "agent entered a startup failure loop; automatic retries are disabled"
                        );
                        return;
                    }
                }
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(retry_delay) => {
                retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
            }
            // Something outside this loop made the provider usable, so the backoff earned by the
            // previous failures no longer describes how likely the next attempt is to succeed.
            _ = wake.recv() => retry_delay = INITIAL_RETRY_DELAY,
            _ = shutdown.recv() => return,
        }
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
    plugin_host: &Arc<PluginApi>,
    home_directory: &Path,
) -> Result<SharedProcess, StartFailure> {
    let StartedAgent {
        process,
        transport,
        messages,
        models,
    } = match source {
        AgentSource::Cli(agent_cli) => spawn_cli_connection(*agent_cli, home_directory).await?,
        AgentSource::Plugin { plugin_id, .. } => {
            spawn_plugin_connection(plugin_id, plugin_host, home_directory).await?
        }
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
            process.terminate_and_reap().await;
            return Err(StartFailure::Retryable(map_acp_error(error)));
        }
        Err(_) => {
            process.terminate_and_reap().await;
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

/// Attaches to one lifecycle-owned agent plugin and wires ACP over its notification channel.
async fn spawn_plugin_connection(
    plugin_id: &PluginId,
    plugin_host: &Arc<PluginApi>,
    home_directory: &Path,
) -> Result<StartedAgent, StartFailure> {
    let attachment = plugin_host
        .attach_agent(plugin_id)
        .await
        .map_err(plugin_attach_error)?;
    let LaunchedPluginAgent { runtime, messages } = match plugin_agent::attach(
        attachment,
        &plugin_id.canonical(),
        home_directory,
        env!("CARGO_PKG_VERSION"),
    )
    .await
    {
        Ok(launched) => launched,
        Err(error) => {
            stop_plugin_runtime(plugin_host, plugin_id).await;
            return Err(plugin_start_error(error));
        }
    };
    let models = match plugin_agent::list_models(&runtime).await {
        Ok(models) => models,
        Err(error) => {
            plugin_agent::stop_agent(&runtime, &plugin_id.canonical()).await;
            stop_plugin_runtime(plugin_host, plugin_id).await;
            return Err(plugin_start_error(error));
        }
    };
    let transport = AgentTransport::Plugin(PluginAcpTransport::new(runtime.clone()));
    Ok(StartedAgent {
        process: AgentProcess::Plugin {
            plugin_id: plugin_id.clone(),
            runtime,
            host: plugin_host.clone(),
        },
        transport,
        messages,
        models,
    })
}

/// Maps a lifecycle refusal to start a plugin onto the supervisor's retry classification.
///
/// A plugin the user has not enabled, or has uninstalled, is an expected local configuration and
/// is reported exactly like a missing CLI so the supervisor retries it without logging: the
/// settings surface already tells the user why that agent is unavailable.
fn plugin_attach_error(error: ConnectionError) -> StartFailure {
    match error {
        ConnectionError::Disabled | ConnectionError::NotFound | ConnectionError::NoProcess => {
            StartFailure::Retryable(runtime_internal(
                "agent_cli_not_found",
                "the plugin behind this agent is not available",
            ))
        }
        ConnectionError::Failed(_)
        | ConnectionError::Timeout
        | ConnectionError::NotReady
        | ConnectionError::NotRunning => {
            StartFailure::Retryable(runtime_internal("agent_start_failed", error.to_string()))
        }
    }
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
fn mark_running_sessions_stopped(pool: &RepositoryPool, clock: SystemClock, agent_ref: &AgentRef) {
    let repository = SqliteSessionRepository::new(pool.clone());
    let Ok(sessions) = repository.list_sessions() else {
        return;
    };
    for session in sessions {
        if session.agent_ref == *agent_ref && session.status == SessionStatus::Running {
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
    use super::{
        AgentSource, ConnectionError, ConnectionSupervisors, PluginAgentError, StartFailure,
        plugin_attach_error, plugin_start_error, resolve_supervised_agents, spawn_runtime_thread,
    };
    use crate::app_event::AppEventHub;
    use crate::clock::SystemClock;
    use crate::plugin::PluginApi;
    use ora_contracts::{PublicError, ScanPluginsRequest};
    use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
    use ora_domain::{AgentCli, AgentRef, PluginId};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Builds one agent plugin source for a package published under the official namespace.
    fn plugin_source(package_name: &str) -> AgentSource {
        // Identity resolution reads only the agent identity; a blank one still needs a
        // well-formed package address, so the fixture falls back to a fixed address.
        let plugin_id = PluginId::new("official", package_name)
            .unwrap_or_else(|_| PluginId::new("official", "fixture").expect("plugin id"));
        AgentSource::Plugin {
            plugin_id,
            package_name: package_name.to_string(),
        }
    }

    /// Verifies a plugin is supervised under the agent identity it supplies, not its package
    /// address.
    ///
    /// A package is addressed as `namespace/name` on disk and in the marketplace, while the agent
    /// it contributes is persisted by every session as the package name alone. Keying supervisors
    /// by the package address instead would make each stored `agent_ref` unresolvable and report
    /// an installed plugin's agent as not installed.
    #[test]
    fn supervises_a_plugin_under_its_agent_identity_rather_than_its_package_address() {
        let resolved = resolve_supervised_agents(
            [AgentSource::Plugin {
                plugin_id: PluginId::new("official", "ora-space.opencode").expect("plugin id"),
                package_name: "ora-space.opencode".to_string(),
            }]
            .into_iter(),
        );

        assert_eq!(
            resolved
                .into_iter()
                .map(|(agent_ref, _source)| agent_ref)
                .collect::<Vec<_>>(),
            vec![AgentRef::parse("ora-space.opencode").expect("parse plugin identity")]
        );
    }

    /// Verifies a plugin-provided agent is supervised under its own package identity.
    ///
    /// This is what makes the agent set open: the identity comes from the installed package
    /// rather than from a set fixed when Ora was built.
    #[test]
    fn supervises_a_plugin_agent_alongside_the_built_in_clis() {
        let resolved = resolve_supervised_agents(
            [
                AgentSource::Cli(AgentCli::Claude),
                plugin_source("acme.my-agent"),
            ]
            .into_iter(),
        );

        assert_eq!(
            resolved
                .into_iter()
                .map(|(agent_ref, _source)| agent_ref)
                .collect::<Vec<_>>(),
            vec![
                AgentCli::Claude.agent_ref(),
                AgentRef::parse("acme.my-agent").expect("parse plugin identity"),
            ]
        );
    }

    /// Verifies a plugin cannot take over an identity another source already supervises.
    #[test]
    fn refuses_a_plugin_that_shadows_an_installed_identity() {
        let resolved = resolve_supervised_agents(
            [
                AgentSource::Cli(AgentCli::Claude),
                plugin_source(AgentCli::Claude.agent_ref().as_str()),
                plugin_source("acme.my-agent"),
            ]
            .into_iter(),
        );

        assert_eq!(
            resolved
                .into_iter()
                .map(|(agent_ref, source)| (agent_ref, matches!(source, AgentSource::Cli(_))))
                .collect::<Vec<_>>(),
            vec![
                (AgentCli::Claude.agent_ref(), true),
                (
                    AgentRef::parse("acme.my-agent").expect("parse plugin identity"),
                    false
                ),
            ]
        );
    }

    /// Verifies a package whose identity is unusable is dropped rather than supervised blindly.
    #[test]
    fn drops_a_source_whose_identity_is_unusable() {
        let resolved = resolve_supervised_agents([plugin_source("   ")].into_iter());

        assert!(resolved.is_empty());
    }

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

    /// Verifies a disabled plugin is reported exactly like a CLI the user has not installed.
    ///
    /// This is what keeps the retry loop silent for a plugin the user simply turned off: the
    /// supervisor logs only genuine startup failures.
    #[test]
    fn treats_a_disabled_plugin_like_a_missing_cli() {
        let failure = plugin_attach_error(ConnectionError::Disabled);

        let StartFailure::Retryable(error) = failure else {
            panic!("a disabled plugin must stay retryable");
        };
        assert!(matches!(
            error.public_error(),
            PublicError::AgentCliNotFound(_)
        ));
    }

    /// Writes one minimal agent package into the plugin root a lifecycle discovers.
    fn write_plugin_package(data_directory: &Path, package_name: &str) {
        let package_root = data_directory
            .join("plugins")
            .join("installed")
            .join("official")
            .join(package_name);
        let package_root = package_root.join("1.0.0");
        fs::create_dir_all(&package_root).expect("create plugin package");
        fs::write(package_root.join("main.js"), "export {};\n").expect("write plugin entrypoint");
        fs::write(
            package_root.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = {package_name:?}\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Example\"\n"
            ),
        )
        .expect("write plugin manifest");
    }

    /// Verifies a package installed after startup is supervised without restarting the host.
    ///
    /// The supervised set was previously fixed when the backend opened, so a plugin installed
    /// while Ora ran appeared in settings but was reported as not installed by every chat until
    /// the next restart. Nothing here starts a plugin process: a freshly discovered package is
    /// disabled, and this asserts only that its agent became reachable.
    #[tokio::test]
    async fn supervises_a_package_that_appears_after_startup() {
        let temporary = TempDir::new().expect("create supervisor test directory");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temporary.path().join("ora.sqlite3")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("create repository pool");
        let plugin_host = Arc::new(
            PluginApi::open(
                pool.clone(),
                temporary.path().to_path_buf(),
                PathBuf::from("deno"),
                SystemClock,
                AppEventHub::new().publisher(),
            )
            .expect("open plugin host"),
        );
        let supervisors = ConnectionSupervisors::start(
            plugin_host.clone(),
            pool,
            temporary.path().to_path_buf(),
            SystemClock,
        );
        let supervised = |supervisors: &ConnectionSupervisors| {
            supervisors
                .statuses()
                .into_iter()
                .map(|(agent_ref, _status)| agent_ref)
                .collect::<Vec<_>>()
        };
        // Supervisors are enumerated in identity order rather than in the order they were offered.
        let mut built_in = AgentCli::ALL
            .into_iter()
            .map(|agent_cli| agent_cli.agent_ref())
            .collect::<Vec<_>>();
        built_in.sort();
        assert_eq!(supervised(&supervisors), built_in);

        write_plugin_package(temporary.path(), "example");
        plugin_host
            .scan(ScanPluginsRequest {})
            .await
            .expect("scan plugins");
        supervisors.sync_plugin_agents();

        let mut expected = built_in;
        expected.push(AgentRef::parse("example").expect("parse plugin identity"));
        expected.sort();
        assert_eq!(supervised(&supervisors), expected);
    }

    /// Verifies a plugin process that refused to start is retried as a genuine failure.
    #[test]
    fn retries_a_plugin_whose_runtime_could_not_launch() {
        let failure = plugin_attach_error(ConnectionError::Failed("deno is missing".to_string()));

        let StartFailure::Retryable(error) = failure else {
            panic!("a failed launch must stay retryable");
        };
        assert!(matches!(
            error.public_error(),
            PublicError::InternalError(_)
        ));
    }
}
