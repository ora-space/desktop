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
use ora_acp::{AcpClient, AcpInboundEvent, AcpPeer};
use ora_application::{Clock, SessionRepository};
use ora_contracts::PublicError;
use ora_db::{RepositoryPool, SqliteSessionRepository};
use ora_domain::{AgentCli, SessionStatus};
use ora_logging::{ora_error, ora_info, ora_warn};
use ora_process::{
    ManagedProcess, ProcessSpawner, ProcessSpec, TokioManagedProcess, TokioProcessSpawner,
};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

/// Exposes one initialized ACP connection without transferring child-process ownership.
#[derive(Clone)]
pub(super) struct RuntimeConnection {
    pub client: AcpClient<ChildStdin>,
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
    agent_cli: AgentCli,
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
    agent_cli: AgentCli,
    state: watch::Receiver<ConnectionState>,
    active_generation: Arc<AtomicU64>,
    routes: Arc<RouteRegistry>,
    shutdown: mpsc::UnboundedSender<()>,
}

/// Owns one independently supervised connection for every supported CLI.
#[derive(Clone)]
pub(super) struct ConnectionSupervisors {
    opencode: ConnectionSupervisor,
    nga: ConnectionSupervisor,
    code_agent_cli: ConnectionSupervisor,
    claude: ConnectionSupervisor,
    codex: ConnectionSupervisor,
}

impl ConnectionSupervisors {
    /// Starts every CLI eagerly so availability is independent across providers.
    pub fn start(pool: RepositoryPool, home_directory: PathBuf, clock: SystemClock) -> Self {
        Self {
            opencode: ConnectionSupervisor::start(
                AgentCli::OpenCode,
                pool.clone(),
                home_directory.clone(),
                clock,
            ),
            nga: ConnectionSupervisor::start(
                AgentCli::Nga,
                pool.clone(),
                home_directory.clone(),
                clock,
            ),
            code_agent_cli: ConnectionSupervisor::start(
                AgentCli::CodeAgentCli,
                pool.clone(),
                home_directory.clone(),
                clock,
            ),
            claude: ConnectionSupervisor::start(
                AgentCli::Claude,
                pool.clone(),
                home_directory.clone(),
                clock,
            ),
            codex: ConnectionSupervisor::start(AgentCli::Codex, pool, home_directory, clock),
        }
    }

    /// Selects the sole application-scoped connection for one persisted CLI identity.
    pub fn for_agent(&self, agent_cli: AgentCli) -> ConnectionSupervisor {
        match agent_cli {
            AgentCli::OpenCode => self.opencode.clone(),
            AgentCli::Nga => self.nga.clone(),
            AgentCli::CodeAgentCli => self.code_agent_cli.clone(),
            AgentCli::Claude => self.claude.clone(),
            AgentCli::Codex => self.codex.clone(),
        }
    }
}

impl ConnectionSupervisor {
    /// Buffers otherwise-unrouted updates until `session/new` returns its provider id.
    pub fn begin_session_setup(&self) -> super::routing::SetupRegistration {
        self.routes.begin_session_setup()
    }

    /// Starts one application-scoped CLI supervisor independently of the caller's runtime.
    pub(super) fn start(
        agent_cli: AgentCli,
        pool: RepositoryPool,
        home_directory: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let (state_sender, state) = watch::channel(ConnectionState::Unavailable);
        let (shutdown, shutdown_receiver) = mpsc::unbounded_channel();
        let active_generation = Arc::new(AtomicU64::new(0));
        let routes = Arc::new(RouteRegistry::default());
        if let Err(error) = spawn_runtime_thread(
            agent_cli,
            run_supervisor(SupervisorContext {
                agent_cli,
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
                agent_cli = agent_cli.database_value(),
                error = %error,
                "agent CLI supervisor thread could not start"
            );
        }
        Self {
            agent_cli,
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
            ConnectionState::Starting | ConnectionState::Unavailable => {
                let executable_name = self.agent_cli.executable_name();
                Err(runtime_internal(
                    "agent_runtime_unavailable",
                    format!("{executable_name} runtime is unavailable"),
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
                format!(
                    "{executable_name} runtime is recovering",
                    executable_name = self.agent_cli.executable_name()
                ),
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
                format!(
                    "{executable_name} runtime is recovering",
                    executable_name = self.agent_cli.executable_name()
                ),
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
fn spawn_runtime_thread<Supervisor>(
    agent_cli: AgentCli,
    supervisor: Supervisor,
) -> std::io::Result<()>
where
    Supervisor: Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name(format!(
            "ora-{executable_name}-supervisor",
            executable_name = agent_cli.executable_name()
        ))
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    ora_error!(
                        agent_cli = agent_cli.database_value(),
                        error = %error,
                        "agent CLI supervisor runtime could not start"
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

struct SharedProcess {
    child: TokioManagedProcess,
    client: AcpClient<ChildStdin>,
    inbound: mpsc::UnboundedReceiver<AcpInboundEvent>,
    load_session_supported: bool,
    list_session_supported: bool,
    close_session_supported: bool,
    delete_session_supported: bool,
}

/// Supervises one process generation at a time and retries only after it is fully reaped.
async fn run_supervisor(context: SupervisorContext) {
    let SupervisorContext {
        agent_cli,
        pool,
        home_directory,
        clock,
        state,
        active_generation,
        routes,
        mut shutdown,
    } = context;
    let mut retry_delay = INITIAL_RETRY_DELAY;
    let mut generation = 0_u64;
    loop {
        let _ = state.send(ConnectionState::Starting);
        match spawn_initialized_process(agent_cli, &home_directory).await {
            Ok(mut process) => {
                generation += 1;
                retry_delay = INITIAL_RETRY_DELAY;
                active_generation.store(generation, Ordering::Release);
                let connection = RuntimeConnection {
                    client: process.client.clone(),
                    generation,
                    load_session_supported: process.load_session_supported,
                    list_session_supported: process.list_session_supported,
                    close_session_supported: process.close_session_supported,
                    delete_session_supported: process.delete_session_supported,
                };
                let _ = state.send(ConnectionState::Ready(connection));
                ora_info!(
                    agent_cli = agent_cli.database_value(),
                    generation,
                    process_id = process.child.id(),
                    "agent CLI runtime is ready"
                );
                let shutting_down =
                    run_process_generation(&mut process, &routes, &mut shutdown).await;
                active_generation.store(0, Ordering::Release);
                let _ = state.send(ConnectionState::Unavailable);
                let error =
                    runtime_internal("agent_runtime_unavailable", "agent CLI connection was lost");
                routes.fail_generation(generation, error);
                mark_running_sessions_stopped(&pool, clock, agent_cli);
                if shutting_down {
                    stop_process_with_grace(&process.child).await;
                    return;
                }
                terminate_and_reap(&process.child).await;
                ora_warn!(
                    agent_cli = agent_cli.database_value(),
                    generation,
                    "agent CLI connection failed; scheduling restart"
                );
            }
            Err(error) => {
                let _ = state.send(ConnectionState::Unavailable);
                // A CLI that is simply not installed is an expected local configuration, and the
                // supervisor keeps retrying it for the whole process lifetime. Logging it would
                // flood the runtime log with one line per retry while `ConnectionState::Unavailable`
                // already carries that fact to the UI, so only genuine startup failures are logged.
                if !matches!(error.public_error(), PublicError::AgentCliNotFound(_)) {
                    ora_warn!(
                        agent_cli = agent_cli.database_value(),
                        error = %error,
                        "agent CLI startup failed; scheduling retry"
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
                            "agent CLI ACP connection failed"
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

/// Starts one CLI in the neutral home directory and completes the ACP handshake.
async fn spawn_initialized_process(
    agent_cli: AgentCli,
    home_directory: &Path,
) -> Result<SharedProcess, BackendError> {
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
    let Some(stdin) = child.take_stdin() else {
        terminate_and_reap(&child).await;
        return Err(runtime_internal(
            "agent_start_failed",
            "agent CLI stdin is unavailable",
        ));
    };
    let Some(stdout) = child.take_stdout() else {
        terminate_and_reap(&child).await;
        return Err(runtime_internal(
            "agent_start_failed",
            "agent CLI stdout is unavailable",
        ));
    };
    if let Some(stderr) = child.take_stderr() {
        tokio::spawn(super::drain_stderr(stderr));
    }
    let peer = AcpPeer::spawn(stdout, stdin);
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
            terminate_and_reap(&child).await;
            return Err(map_acp_error(error));
        }
        Err(_) => {
            terminate_and_reap(&child).await;
            return Err(runtime_internal(
                "agent_initialize_timeout",
                "agent CLI initialization timed out",
            ));
        }
    };
    let (client, inbound) = peer.into_parts();
    Ok(SharedProcess {
        child,
        client,
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

/// Persists one CLI's connection loss without stopping sessions owned by healthy CLIs.
fn mark_running_sessions_stopped(pool: &RepositoryPool, clock: SystemClock, agent_cli: AgentCli) {
    let repository = SqliteSessionRepository::new(pool.clone());
    let Ok(sessions) = repository.list_sessions() else {
        return;
    };
    for session in sessions {
        if session.agent_cli == agent_cli && session.status == SessionStatus::Running {
            let _ = repository.update_session_status(
                &session.id,
                SessionStatus::Stopped,
                clock.now_timestamp_millis(),
            );
        }
    }
}

/// Reaps a failed process before replacement so two generations of one CLI cannot overlap.
async fn terminate_and_reap(child: &TokioManagedProcess) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// Bounds application shutdown even when the operating system does not promptly reap the child.
async fn stop_process_with_grace(child: &TokioManagedProcess) {
    let _ = timeout(CANCELLATION_GRACE, async {
        let _ = child.kill().await;
        let _ = child.wait().await;
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::spawn_runtime_thread;
    use ora_domain::AgentCli;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    /// Verifies synchronous bootstrap can launch async supervision without an ambient runtime.
    #[test]
    fn starts_a_dedicated_runtime_thread() {
        let (sender, receiver) = std::sync::mpsc::channel();

        spawn_runtime_thread(AgentCli::OpenCode, async move {
            sender.send("ready").expect("send runtime signal");
        })
        .expect("start runtime thread");

        assert_eq!(receiver.recv_timeout(Duration::from_secs(1)), Ok("ready"));
    }
}
