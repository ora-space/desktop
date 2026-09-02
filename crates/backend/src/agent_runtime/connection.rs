use super::plugin_agent::{self, LaunchedPluginAgent, PluginAcpTransport, PluginAgentError};
use super::restart_circuit::{RestartCircuit, RestartDecision};
use super::routing::{RouteRegistry, SessionChannel, SessionEvent};
use super::{
    CANCELLATION_GRACE, CONTRACT_QUEUE_CAPACITY, INITIALIZE_TIMEOUT, map_acp_error,
    runtime_internal,
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
use ora_acp::{AcpClient, AcpInboundEvent, AcpMessages, AcpPeer};
use ora_application::{Clock, SessionRepository};
use ora_contracts::{
    InstalledPluginContribution, ListInstalledPluginsRequest, PublicError, StopPluginRequest,
};
use ora_db::{RepositoryPool, SqliteSessionRepository};
use ora_domain::{AgentRef, PluginId, SessionStatus};
use ora_logging::{ora_error, ora_info, ora_warn};
use ora_plugin_lifecycle::ConnectionError;
use ora_plugin_runtime::{PluginProcessExit, PluginRuntime};
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

/// Names the ACP client every supervised agent connection speaks through.
///
/// `RuntimeConnection` is published through a `watch` channel, so the transport cannot stay
/// generic. Every agent is supplied by a plugin, so there is exactly one transport to name.
pub(super) type AgentAcpClient = AcpClient<PluginAcpTransport>;

/// Returns the agent identity one installed package supplies.
///
/// The identity is the package's whole canonical plugin id, namespace included. Using only the
/// name segment would collapse two packages that different marketplace sources published under
/// the same `identifier` into a single agent: the supervisor map is keyed by this value, so one
/// of the two would be dropped, only one would reach the picker, and which one won would depend
/// on the order installed packages happen to be walked. Sessions persist the same full id in
/// `agent_cli`, so a stored binding always resolves back to the package that answered it.
fn agent_identity(plugin_id: &PluginId) -> AgentRef {
    // A canonical plugin id is non-empty by construction, so this cannot fail; parsing keeps one
    // construction path for the value object rather than a second, unchecked one.
    AgentRef::parse(plugin_id.canonical())
        .unwrap_or_else(|error| unreachable!("a canonical plugin id is an agent ref: {error}"))
}

/// Exposes one initialized ACP connection without transferring child-process ownership.
#[derive(Clone)]
pub(super) struct RuntimeConnection {
    pub client: AgentAcpClient,
    /// The plugin control channel used for on-demand capabilities outside ACP.
    pub runtime: PluginRuntime,
    pub generation: u64,
    pub load_session_supported: bool,
    /// Whether initialize advertised ACP HTTP MCP servers.
    pub http_mcp_supported: bool,
    /// Whether the agent advertises the bounded fallback used for first-title acquisition.
    pub list_session_supported: bool,
    pub close_session_supported: bool,
    /// Whether the agent advertises `session/delete`.
    ///
    /// Failed starts Ora created but never handed to the user are removed with
    /// it so unused provider history does not accumulate; agents without it fall
    /// back to `session/close`, which only detaches.
    pub delete_session_supported: bool,
}

#[derive(Clone)]
enum ConnectionState {
    Starting,
    Ready(RuntimeConnection),
    Unavailable,
    Failing,
}

/// Reports one agent's live detection state without exposing its private connection handle.
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
    plugin_id: PluginId,
    /// Starts and stops the processes behind plugin-provided agents.
    plugin_host: Arc<PluginApi>,
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
/// Agents are keyed by their persisted namespaced identity rather than by a closed enum, because
/// every agent is supplied by an installed plugin and which ones exist is not known at build time.
///
/// The set is mutable because installing a plugin adds an agent while Ora is running. It is held
/// behind a lock rather than rebuilt, so every clone held by a live session actor observes an
/// install or uninstall without being handed a new value.
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
    /// Starts one supervisor per installed agent plugin eagerly.
    ///
    /// Availability stays independent per agent: one provider that is missing or crash-looping
    /// never delays or degrades the others, which is why each gets its own supervisor.
    ///
    /// Every installed agent plugin is supervised; the lifecycle starts its process on demand.
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
            .filter_map(|plugin| PluginId::parse(&plugin.id).ok());
        // Every installed package has a distinct id, so no two agents can claim one identity and
        // there is nothing to arbitrate: the desired set is exactly the installed set.
        let desired = agent_plugins
            .map(|plugin_id| (agent_identity(&plugin_id), plugin_id))
            .collect::<Vec<_>>();

        let mut supervisors = self
            .supervisors
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let desired_refs = desired
            .iter()
            .map(|(agent_ref, _plugin_id)| agent_ref.clone())
            .collect::<BTreeSet<_>>();
        // Dropping the map's handle only signals shutdown once every session actor holding a clone
        // has released it, so an uninstall never severs a conversation that is still open.
        supervisors.retain(|agent_ref, _supervisor| desired_refs.contains(agent_ref));
        for (agent_ref, plugin_id) in desired {
            if supervisors.contains_key(&agent_ref) {
                continue;
            }
            let supervisor = ConnectionSupervisor::start(
                agent_ref.clone(),
                plugin_id,
                self.plugin_host.clone(),
                self.pool.clone(),
                self.home_directory.clone(),
                self.clock,
            );
            supervisors.insert(agent_ref, supervisor);
        }
    }

    /// Resolves a plugin package address onto the agent identity its sessions are bound to.
    ///
    /// The two are the same value now that an agent is identified by its whole plugin id, but the
    /// lookup remains because the answer also has to say whether that package is installed: a
    /// caller holding a package address wants the agent it currently supplies, not an identity
    /// minted for a package that is gone.
    pub fn agent_for_plugin(&self, plugin_id: &PluginId) -> Option<AgentRef> {
        self.plugin_host
            .list(ListInstalledPluginsRequest {})
            .plugins
            .iter()
            .any(|plugin| plugin.id == plugin_id.canonical())
            .then(|| agent_identity(plugin_id))
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
    /// Enumerating what is actually supervised is what lets an agent appear in the picker: the
    /// set is decided by which packages are installed, not by the build.
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
        plugin_id: PluginId,
        plugin_host: Arc<PluginApi>,
        pool: RepositoryPool,
        home_directory: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let (state_sender, state) = watch::channel(ConnectionState::Unavailable);
        let (shutdown, shutdown_receiver) = mpsc::unbounded_channel();
        let active_generation = Arc::new(AtomicU64::new(0));
        let routes = Arc::new(RouteRegistry::default());
        let label: Arc<str> = Arc::from(plugin_id.name());
        let identifier = agent_ref.to_string();
        if let Err(error) = spawn_runtime_thread(
            &label,
            run_supervisor(SupervisorContext {
                agent_ref,
                plugin_id,
                plugin_host,
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

/// Ends the plugin process backing one connection generation once that generation is over.
///
/// The process belongs to the plugin lifecycle rather than to this module: a connection only
/// borrowed its ACP stream, so ending the generation means telling the lifecycle to stop it, which
/// keeps the runtime state the settings surface reports honest and leaves the next attach to start
/// a fresh process.
struct AgentProcess {
    plugin_id: PluginId,
    runtime: PluginRuntime,
    host: Arc<PluginApi>,
}

impl AgentProcess {
    /// Reaps a failed generation before its replacement so two generations cannot overlap.
    async fn terminate_and_reap(&self) {
        // Stopping the agent is the plugin's chance to reap the agent process it owns before the
        // lifecycle ends the plugin process itself.
        plugin_agent::stop_agent(&self.runtime, &self.plugin_id.canonical()).await;
        stop_plugin_runtime(&self.host, &self.plugin_id).await;
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
/// Almost every failure is retryable: an agent can be installed later, a crashed provider can come
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
    transport: PluginAcpTransport,
    messages: AcpMessages,
}

struct SharedProcess {
    process: AgentProcess,
    client: AgentAcpClient,
    inbound: mpsc::UnboundedReceiver<AcpInboundEvent>,
    load_session_supported: bool,
    http_mcp_supported: bool,
    list_session_supported: bool,
    close_session_supported: bool,
    delete_session_supported: bool,
}

/// Supervises one process generation at a time and retries only after it is fully reaped.
async fn run_supervisor(context: SupervisorContext) {
    let SupervisorContext {
        agent_ref,
        plugin_id,
        plugin_host,
        pool,
        home_directory,
        clock,
        state,
        active_generation,
        routes,
        mut shutdown,
    } = context;
    let identifier = agent_ref.as_str();
    let mut retry_delay = INITIAL_RETRY_DELAY;
    let mut generation = 0_u64;
    let mut restart_circuit = RestartCircuit::default();
    loop {
        let _ = state.send(ConnectionState::Starting);
        match spawn_initialized_process(&plugin_id, &plugin_host, &home_directory).await {
            Ok(mut process) => {
                generation += 1;
                retry_delay = INITIAL_RETRY_DELAY;
                active_generation.store(generation, Ordering::Release);
                let connection = RuntimeConnection {
                    client: process.client.clone(),
                    runtime: process.process.runtime.clone(),
                    generation,
                    load_session_supported: process.load_session_supported,
                    http_mcp_supported: process.http_mcp_supported,
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
                if !matches!(error.public_error(), PublicError::AgentNotInstalled(_)) {
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
            _ = shutdown.recv() => return,
        }
    }
}

/// Drains and demultiplexes one live connection until shutdown, a transport-level failure, or the
/// backing plugin process dies.
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
            // A dead process never sends another frame, so the inbound stream alone cannot report
            // the loss. Without this watch the supervisor would park here forever on a zombie
            // connection instead of applying its restart policy to the failure it already knows.
            exit = process.process.runtime.wait_for_exit() => match exit {
                PluginProcessExit::Failed(_) => return false,
                PluginProcessExit::Stopped => return true,
            },
            _ = shutdown.recv() => return true,
        }
    }
}

/// Starts one agent in the neutral home directory and completes the ACP handshake.
///
/// The connection is only reported ready once ACP `initialize` has returned its capabilities, so
/// no caller can send a session request to a transport that is not yet carrying a live agent.
async fn spawn_initialized_process(
    plugin_id: &PluginId,
    plugin_host: &Arc<PluginApi>,
    home_directory: &Path,
) -> Result<SharedProcess, StartFailure> {
    let StartedAgent {
        process,
        transport,
        messages,
    } = spawn_plugin_connection(plugin_id, plugin_host, home_directory).await?;
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
        inbound,
        load_session_supported: response.agent_capabilities.load_session,
        http_mcp_supported: response.agent_capabilities.mcp_capabilities.http,
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
    let LaunchedPluginAgent {
        runtime,
        messages,
        effect_declaration,
    } = match plugin_agent::attach(
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
    plugin_host
        .replace_agent_effect_declaration(plugin_id.clone(), effect_declaration)
        .map_err(|error| {
            StartFailure::Terminal(runtime_internal("agent_start_failed", error.to_string()))
        })?;
    let transport = PluginAcpTransport::new(runtime.clone());
    Ok(StartedAgent {
        process: AgentProcess {
            plugin_id: plugin_id.clone(),
            runtime,
            host: plugin_host.clone(),
        },
        transport,
        messages,
    })
}

/// Maps a lifecycle refusal to start a plugin onto the supervisor's retry classification.
///
/// An uninstalled plugin is reported like a missing CLI so the supervisor retries without noisy
/// logging while package discovery catches up.
fn plugin_attach_error(error: ConnectionError) -> StartFailure {
    match error {
        ConnectionError::NotFound | ConnectionError::NoProcess => {
            StartFailure::Retryable(runtime_internal(
                "agent_not_installed",
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

/// Maps a plugin startup failure onto the supervisor's retry classification.
///
/// A plugin whose own agent process is not installed on this machine is an expected local
/// configuration, so it is reported exactly like an uninstalled plugin and retried without
/// logging: the user can install the CLI while Ora keeps running, and the next attempt picks it
/// up. A plugin that reports its own bundled agent as unusable is the opposite case — the same
/// package produces the same failure on every attempt — so it is abandoned like an unservable
/// contract rather than retried behind a quiet `agent_not_installed`.
fn plugin_start_error(error: PluginAgentError) -> StartFailure {
    match error {
        PluginAgentError::AgentNotInstalled => StartFailure::Retryable(runtime_internal(
            "agent_not_installed",
            "the agent behind this plugin is not installed",
        )),
        PluginAgentError::AgentUnusable(detail) => {
            StartFailure::Terminal(runtime_internal("agent_start_failed", detail))
        }
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
        ConnectionError, ConnectionSupervisors, PluginAgentError, StartFailure, agent_identity,
        plugin_attach_error, plugin_start_error, spawn_runtime_thread,
    };
    use crate::app_event::AppEventHub;
    use crate::clock::SystemClock;
    use crate::plugin::PluginApi;
    use crate::user_config::UserConfigApi;
    use ora_contracts::{PublicError, ScanPluginsRequest};
    use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
    use ora_domain::{AgentRef, PluginId};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Verifies an agent is identified by its whole canonical plugin id, namespace included.
    ///
    /// Sessions persist this value in `agent_cli` and the supervisor map is keyed by it, so
    /// dropping the namespace here would make two packages published by different marketplace
    /// sources under one `identifier` collapse into a single agent.
    #[test]
    fn identifies_an_agent_by_its_whole_plugin_id() {
        let plugin_id = PluginId::new("official", "ora-space.opencode").expect("plugin id");

        assert_eq!(
            agent_identity(&plugin_id),
            AgentRef::parse("official/ora-space.opencode").expect("parse plugin identity"),
        );
    }

    /// Verifies two packages sharing an `identifier` across marketplace sources stay two agents.
    ///
    /// Both can be installed, both are supervised, and each keeps the sessions written against
    /// it. Under a name-only identity the two would claim one supervisor slot and the winner
    /// would be decided by the order installed packages are walked, quietly handing one source's
    /// existing conversations to the other source's implementation.
    #[test]
    fn keeps_same_identifier_agents_from_different_sources_distinct() {
        let identities = [
            PluginId::new("official", "acme.agent").expect("plugin id"),
            PluginId::new("plugins.2aa64f48", "acme.agent").expect("plugin id"),
        ]
        .map(|plugin_id| agent_identity(&plugin_id));

        assert_eq!(
            identities.to_vec(),
            vec![
                AgentRef::parse("official/acme.agent").expect("parse plugin identity"),
                AgentRef::parse("plugins.2aa64f48/acme.agent").expect("parse plugin identity"),
            ],
        );
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

    /// Verifies a plugin whose agent is absent stays retryable and reports it as not installed.
    #[test]
    fn treats_a_missing_plugin_agent_as_not_installed() {
        let failure = plugin_start_error(PluginAgentError::AgentNotInstalled);

        let StartFailure::Retryable(error) = failure else {
            panic!("a missing agent must stay retryable");
        };
        assert!(matches!(
            error.public_error(),
            PublicError::AgentNotInstalled(_)
        ));
    }

    /// Verifies a plugin whose bundled agent cannot run is abandoned rather than retried quietly,
    /// carrying the plugin's own detail: giving up means this is the only report it ever produces.
    #[test]
    fn gives_up_on_a_plugin_whose_bundled_agent_is_unusable() {
        let failure = plugin_start_error(PluginAgentError::AgentUnusable(
            "the bundled agent `bin/opencode` cannot run".to_string(),
        ));

        let StartFailure::Terminal(error) = failure else {
            panic!("an unusable bundled agent must not be retried");
        };
        assert_eq!(
            error.to_string(),
            "the bundled agent `bin/opencode` cannot run"
        );
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
    /// disabled, and this asserts only that its agent became reachable. Nothing is supervised
    /// before the install, because no agent exists that an installed package did not supply.
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
                Arc::new(UserConfigApi::new(pool.clone())),
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
        assert_eq!(supervised(&supervisors), Vec::new());

        write_plugin_package(temporary.path(), "example");
        plugin_host
            .scan(ScanPluginsRequest {})
            .await
            .expect("scan plugins");
        supervisors.sync_plugin_agents();

        assert_eq!(
            supervised(&supervisors),
            vec![AgentRef::parse("official/example").expect("parse plugin identity")]
        );
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
