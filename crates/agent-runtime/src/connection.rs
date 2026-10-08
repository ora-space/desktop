mod startup;

#[cfg(test)]
#[path = "prompt_policy_tests.rs"]
mod prompt_policy_tests;

use super::plugin_agent::PluginAcpTransport;
use super::restart_circuit::{RestartCircuit, RestartDecision};
use super::routing::{RouteRegistry, SessionChannel, SessionEvent};
use super::suspend::{SuspendedAgents, is_agent_suspended};
use super::{CONTRACT_QUEUE_CAPACITY, runtime_unavailable_because};
use crate::RuntimeError;
use crate::clock::SystemClock;
use crate::host::SessionStore;
use crate::host::{AgentAttach, AgentRuntimeHost};
use agent_client_protocol_schema::v1::{RequestPermissionOutcome, RequestPermissionResponse};
use ora_acp::{AcpClient, AcpInboundEvent};
use ora_contracts::PublicError;
use ora_domain::{AgentRef, PluginId, SessionStatus};
use ora_logging::{ora_error, ora_info, ora_warn};
use ora_plugin_runtime::{PluginProcessExit, PluginRuntime};
use startup::{SharedProcess, StartFailure, spawn_initialized_process};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

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
struct SupervisorContext<H: AgentRuntimeHost> {
    agent_ref: AgentRef,
    plugin_id: PluginId,
    /// Starts and stops the processes behind plugin-provided agents.
    plugin_host: Arc<H::Attach>,
    store: H::Store,
    home_directory: PathBuf,
    clock: SystemClock,
    state: watch::Sender<ConnectionState>,
    active_generation: Arc<AtomicU64>,
    routes: Arc<RouteRegistry>,
    suspended: SuspendedAgents,
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
pub(super) struct ConnectionSupervisors<H: AgentRuntimeHost> {
    supervisors: Arc<RwLock<BTreeMap<AgentRef, ConnectionSupervisor>>>,
    /// Agent identities whose supervisor is temporarily barred from spawning.
    suspended: SuspendedAgents,
    /// Retained so a package installed after startup can be supervised without restarting Ora.
    plugin_host: Arc<H::Attach>,
    store: H::Store,
    home_directory: PathBuf,
    clock: SystemClock,
}

impl<H: AgentRuntimeHost> Clone for ConnectionSupervisors<H> {
    fn clone(&self) -> Self {
        Self {
            supervisors: Arc::clone(&self.supervisors),
            suspended: Arc::clone(&self.suspended),
            plugin_host: Arc::clone(&self.plugin_host),
            store: self.store.clone(),
            home_directory: self.home_directory.clone(),
            clock: self.clock,
        }
    }
}

impl<H: AgentRuntimeHost> ConnectionSupervisors<H> {
    /// Starts one supervisor per installed agent plugin eagerly.
    ///
    /// Availability stays independent per agent: one provider that is missing or crash-looping
    /// never delays or degrades the others, which is why each gets its own supervisor.
    ///
    /// Every installed agent plugin is supervised; the lifecycle starts its process on demand.
    pub fn start(
        plugin_host: Arc<H::Attach>,
        store: H::Store,
        home_directory: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let supervisors = Self {
            supervisors: Arc::new(RwLock::new(BTreeMap::new())),
            suspended: Arc::new(Mutex::new(BTreeSet::new())),
            plugin_host,
            store,
            home_directory,
            clock,
        };
        supervisors.sync_plugin_agents();
        supervisors
    }

    /// Bars the plugin agent's supervisor from respawning its process.
    ///
    /// Package replacement needs the plugin process gone and gone for good: the supervisor's
    /// normal restart loop would race the replacement by re-attaching to the version directory
    /// that is about to be retired.
    pub(super) fn suspend_plugin_agent(&self, plugin_id: &str) {
        let Ok(plugin_id) = PluginId::parse(plugin_id) else {
            return;
        };
        self.suspended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(AgentRef::for_plugin(&plugin_id));
    }

    /// Lifts a suspension and drops the supervisor so the next reconciliation starts a fresh one
    /// against the package version that is now installed.
    pub(super) fn resume_plugin_agent(&self, plugin_id: &str) {
        let Ok(plugin_id) = PluginId::parse(plugin_id) else {
            return;
        };
        let agent_ref = AgentRef::for_plugin(&plugin_id);
        self.suspended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&agent_ref);
        self.supervisors
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&agent_ref);
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
        let agent_plugins = self.plugin_host.installed_agent_plugins().into_iter();
        // Every installed package has a distinct id, so no two agents can claim one identity and
        // there is nothing to arbitrate: the desired set is exactly the installed set.
        let suspended = Arc::clone(&self.suspended);
        let desired = agent_plugins
            .map(|plugin_id| (AgentRef::for_plugin(&plugin_id), plugin_id))
            .filter(|(agent_ref, _plugin_id)| !is_agent_suspended(&suspended, agent_ref))
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
            let supervisor = ConnectionSupervisor::start::<H>(
                agent_ref.clone(),
                plugin_id,
                Arc::clone(&self.suspended),
                self.plugin_host.clone(),
                self.store.clone(),
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
            .is_installed(plugin_id)
            .then(|| AgentRef::for_plugin(plugin_id))
    }

    /// Selects the sole application-scoped connection for one persisted agent identity.
    ///
    /// A miss is a normal runtime state rather than data corruption: a session can outlive the
    /// plugin that provided its agent, and the caller reports that as an unavailable runtime.
    pub fn for_agent(&self, agent_ref: &AgentRef) -> Result<ConnectionSupervisor, RuntimeError> {
        self.supervisors
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(agent_ref)
            .cloned()
            .ok_or_else(|| runtime_unavailable_because(format!("{agent_ref} is not installed")))
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
    pub(super) fn start<H: AgentRuntimeHost>(
        agent_ref: AgentRef,
        plugin_id: PluginId,
        suspended: SuspendedAgents,
        plugin_host: Arc<H::Attach>,
        store: H::Store,
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
            run_supervisor(SupervisorContext::<H> {
                agent_ref,
                plugin_id,
                suspended,
                plugin_host,
                store,
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

    /// Waits until the connection is ready, or reports that the supervisor gave up on it.
    ///
    /// Startup and retries continue underneath, so `Starting` and `Unavailable` are waited
    /// through; only `Failing`, or a supervisor that exited, is final.
    pub async fn ready(&self) -> Result<(), RuntimeError> {
        let mut state = self.state.clone();
        loop {
            match &*state.borrow_and_update() {
                ConnectionState::Ready(_) => return Ok(()),
                ConnectionState::Failing => {
                    return Err(runtime_unavailable_because(format!(
                        "{label} runtime stopped retrying",
                        label = self.label
                    )));
                }
                ConnectionState::Starting | ConnectionState::Unavailable => {}
            }
            if state.changed().await.is_err() {
                return Err(runtime_unavailable_because(format!(
                    "{label} runtime supervisor exited",
                    label = self.label
                )));
            }
        }
    }

    /// Returns the initialized shared connection or a stable degraded-runtime error.
    pub fn current(&self) -> Result<RuntimeConnection, RuntimeError> {
        match self.state.borrow().clone() {
            ConnectionState::Ready(connection) => Ok(connection),
            ConnectionState::Starting | ConnectionState::Unavailable | ConnectionState::Failing => {
                Err(runtime_unavailable_because(format!(
                    "{label} runtime is unavailable",
                    label = self.label
                )))
            }
        }
    }

    /// Registers a bounded ordered event route and independent failure controls for one session.
    pub fn open_session_channel(
        &self,
        agent_session_id: &str,
        ora_session_id: &str,
    ) -> Result<SessionChannel, RuntimeError> {
        let connection = self.current()?;
        if self.active_generation.load(Ordering::Acquire) != connection.generation {
            return Err(runtime_unavailable_because(format!(
                "{label} runtime is recovering",
                label = self.label
            )));
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
            return Err(runtime_unavailable_because(format!(
                "{label} runtime is recovering",
                label = self.label
            )));
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

/// Supervises one process generation at a time and retries only after it is fully reaped.
async fn run_supervisor<H: AgentRuntimeHost>(context: SupervisorContext<H>) {
    let SupervisorContext {
        agent_ref,
        plugin_id,
        suspended,
        plugin_host,
        store,
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
        // A suspended supervisor never spawns: package replacement holds this flag while it
        // retires the version directory the plugin process keeps open as its working directory.
        if is_agent_suspended(&suspended, &agent_ref) {
            let _ = state.send(ConnectionState::Unavailable);
            return;
        }
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
                let error = runtime_unavailable_because("agent connection was lost");
                routes.fail_generation(generation, error);
                mark_running_sessions_stopped(&store, clock, &agent_ref);
                if shutting_down {
                    process.process.stop_with_grace().await;
                    return;
                }
                process.process.terminate_and_reap().await;
                // The generation lost its process while the supervisor is suspended (package
                // replacement): exiting here is what lets the replacement retire the version
                // directory without this supervisor racing it with a respawn.
                if is_agent_suspended(&suspended, &agent_ref) {
                    let _ = state.send(ConnectionState::Unavailable);
                    return;
                }
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
async fn run_process_generation<A: AgentAttach>(
    process: &mut SharedProcess<A>,
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

/// Persists one agent's connection loss without stopping sessions owned by healthy agents.
fn mark_running_sessions_stopped(
    repository: &impl SessionStore,
    clock: SystemClock,
    agent_ref: &AgentRef,
) {
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
    use super::startup::{StartFailure, plugin_attach_error, plugin_start_error};
    use super::{ConnectionSupervisors, spawn_runtime_thread};
    use crate::MemorySessionStore;
    use crate::clock::SystemClock;
    use crate::plugin_agent::PluginAgentError;
    use crate::test_host::{InstalledAgents, TestHost};
    use ora_contracts::{EmptyErrorParams, PublicError};
    use ora_domain::{AgentRef, PluginId};
    use ora_plugin_lifecycle::ConnectionError;
    use pretty_assertions::assert_eq;
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
            AgentRef::for_plugin(&plugin_id),
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
        .map(|plugin_id| AgentRef::for_plugin(&plugin_id));

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

    /// Verifies a package installed after startup is supervised without restarting the host.
    ///
    /// The supervised set was previously fixed when the host opened, so a plugin installed
    /// while Ora ran appeared in settings but was reported as not installed by every chat until
    /// the next restart. Nothing here starts a plugin process: a freshly discovered package is
    /// disabled, and this asserts only that its agent became reachable. Nothing is supervised
    /// before the install, because no agent exists that an installed package did not supply.
    #[tokio::test]
    async fn supervises_a_package_that_appears_after_startup() {
        let temporary = TempDir::new().expect("create supervisor test directory");
        let plugin_host = Arc::new(InstalledAgents::default());
        let supervisors = ConnectionSupervisors::<TestHost>::start(
            plugin_host.clone(),
            MemorySessionStore::default(),
            temporary.path().to_path_buf(),
            SystemClock,
        );
        let supervised = |supervisors: &ConnectionSupervisors<TestHost>| {
            supervisors
                .statuses()
                .into_iter()
                .map(|(agent_ref, _status)| agent_ref)
                .collect::<Vec<_>>()
        };
        assert_eq!(supervised(&supervisors), Vec::new());

        plugin_host.install(PluginId::new("official", "example").expect("plugin id"));
        supervisors.sync_plugin_agents();

        assert_eq!(
            supervised(&supervisors),
            vec![AgentRef::parse("official/example").expect("parse plugin identity")]
        );
    }

    /// Verifies a plugin process that refused to start is retried as a genuine failure.
    ///
    /// The public error must name the startup failure rather than collapse into `InternalError`:
    /// a missing runtime is the user's own local setup, so the toast has to say so instead of
    /// asking them to report a request ID for a defect Ora did not cause.
    #[test]
    fn retries_a_plugin_whose_runtime_could_not_launch() {
        let failure = plugin_attach_error(ConnectionError::Failed("deno is missing".to_string()));

        let StartFailure::Retryable(error) = failure else {
            panic!("a failed launch must stay retryable");
        };
        assert_eq!(
            error.public_error(),
            &PublicError::AgentStartFailed(EmptyErrorParams {})
        );
    }

    /// Verifies a lifecycle deadline is reported consistently with every other runtime timeout.
    #[test]
    fn reports_a_plugin_start_timeout_as_timed_out() {
        let failure = plugin_attach_error(ConnectionError::Timeout);

        let StartFailure::Retryable(error) = failure else {
            panic!("a plugin start timeout must stay retryable");
        };
        assert_eq!(
            error.public_error(),
            &PublicError::AgentTimedOut(EmptyErrorParams {})
        );
    }
}
