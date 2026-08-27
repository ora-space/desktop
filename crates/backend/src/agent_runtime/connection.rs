use super::plugin_agent::{
    self, LaunchedPluginAgent, PluginAcpTransport, PluginAgentError, PluginAgentModel,
};
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
use ora_plugin_runtime::PluginRuntime;
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

/// Identifies the installed plugin package that supplies one supervised agent.
///
/// A package carries two distinct identities and both are needed here. `plugin_id` is the
/// package address — `namespace/name` — that the plugin lifecycle owns the process under.
/// `package_name` is the agent identity the package supplies, which is what sessions persist as
/// their `agent_ref`. Supervising a plugin under its package address instead would leave every
/// stored binding pointing at an agent no lookup can reach.
#[derive(Debug, Clone)]
pub(super) struct AgentSource {
    pub plugin_id: PluginId,
    pub package_name: String,
}

impl AgentSource {
    /// Returns the persisted, namespaced identity of the agent this source provides.
    fn agent_ref(&self) -> Result<AgentRef, BackendError> {
        // A package name reaches here already validated by discovery, but parsing keeps one
        // construction path for the value object rather than a second, unchecked one.
        AgentRef::parse(&self.package_name)
            .map_err(|error| runtime_internal("agent_start_failed", error.to_string()))
    }

    /// Returns the short name used for supervisor thread names and operator-facing messages.
    fn label(&self) -> &str {
        self.plugin_id.name()
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
            .filter_map(|plugin| {
                PluginId::parse(&plugin.id)
                    .ok()
                    .map(|plugin_id| AgentSource {
                        plugin_id,
                        package_name: plugin.name,
                    })
            });
        let desired = resolve_supervised_agents(agent_plugins);

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

    /// Resolves a plugin package address onto the agent identity its sessions are bound to.
    ///
    /// A package carries two identities (see [`AgentSource::Plugin`]): the address the plugin
    /// lifecycle owns the process under, and the agent name a Session persists as its `agent_ref`.
    /// A caller holding the first cannot compare it against the second — they are different
    /// strings, and comparing them directly matches nothing at all while looking perfectly
    /// reasonable. The translation lives here, beside the declaration that owns both halves.
    pub fn agent_for_plugin(&self, plugin_id: &PluginId) -> Option<AgentRef> {
        self.plugin_host
            .list(ListInstalledPluginsRequest {})
            .plugins
            .into_iter()
            .find(|plugin| plugin.id == plugin_id.canonical())
            .and_then(|plugin| AgentRef::parse(plugin.name).ok())
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
        source: AgentSource,
        plugin_host: Arc<PluginApi>,
        pool: RepositoryPool,
        home_directory: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let (state_sender, state) = watch::channel(ConnectionState::Unavailable);
        let (shutdown, shutdown_receiver) = mpsc::unbounded_channel();
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

/// Decides which agent identity each source supervises, in the order the sources were offered.
///
/// A package that claims an identity another package already took is dropped rather than allowed
/// to replace it: silently handing a user's existing agent to a different package is worse than
/// ignoring the second one. A source whose identity is unusable is dropped for the same reason —
/// there would be no way to address it.
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
/// The connection is only reported ready once ACP `initialize` has returned its capabilities, so
/// no caller can send a session request to a transport that is not yet carrying a live agent.
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
    } = spawn_plugin_connection(&source.plugin_id, plugin_host, home_directory).await?;
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
        effect_surfaces,
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
        .replace_agent_effect_surfaces(plugin_id.clone(), effect_surfaces)
        .map_err(|error| {
            StartFailure::Terminal(runtime_internal("agent_start_failed", error.to_string()))
        })?;
    let models = match plugin_agent::list_models(&runtime).await {
        Ok(models) => models,
        Err(error) => {
            plugin_agent::stop_agent(&runtime, &plugin_id.canonical()).await;
            stop_plugin_runtime(plugin_host, plugin_id).await;
            return Err(plugin_start_error(error));
        }
    };
    let transport = PluginAcpTransport::new(runtime.clone());
    Ok(StartedAgent {
        process: AgentProcess {
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
/// logging.
fn plugin_start_error(error: PluginAgentError) -> StartFailure {
    match error {
        PluginAgentError::AgentNotInstalled => StartFailure::Retryable(runtime_internal(
            "agent_not_installed",
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

    /// Builds one agent plugin source for a package published under the official namespace.
    fn plugin_source(package_name: &str) -> AgentSource {
        // Identity resolution reads only the agent identity; a blank one still needs a
        // well-formed package address, so the fixture falls back to a fixed address.
        let plugin_id = PluginId::new("official", package_name)
            .unwrap_or_else(|_| PluginId::new("official", "fixture").expect("plugin id"));
        AgentSource {
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
            [AgentSource {
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

    /// Verifies several installed packages are each supervised under their own identity.
    ///
    /// This is what makes the agent set open: every identity comes from an installed package
    /// rather than from a set fixed when Ora was built.
    #[test]
    fn supervises_every_installed_agent_package() {
        let resolved = resolve_supervised_agents(
            [
                plugin_source("ora-space.claude"),
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
                AgentRef::parse("ora-space.claude").expect("parse plugin identity"),
                AgentRef::parse("acme.my-agent").expect("parse plugin identity"),
            ]
        );
    }

    /// Verifies a package cannot take over an identity another package already supervises.
    #[test]
    fn refuses_a_plugin_that_shadows_an_installed_identity() {
        let resolved = resolve_supervised_agents(
            [
                plugin_source("ora-space.claude"),
                plugin_source("ora-space.claude"),
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
                AgentRef::parse("ora-space.claude").expect("parse plugin identity"),
                AgentRef::parse("acme.my-agent").expect("parse plugin identity"),
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
            vec![AgentRef::parse("example").expect("parse plugin identity")]
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
