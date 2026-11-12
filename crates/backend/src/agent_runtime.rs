//! Desktop host for the shared ACP agent runtime.
//!
//! `ora-agent-runtime` owns every session semantic; this module supplies what only Desktop knows:
//! session rows in SQLite, agent plugins reached through [`PluginApi`], Session MCP resolved from
//! installed packages and their Settings, application events, and Workspace directories resolved
//! against the bootstrap path base. Runtime failures cross back into Desktop as [`BackendError`]
//! unchanged, so callers observe exactly the errors they did before the runtime was shared.

#[cfg(test)]
mod host_tests;
#[cfg(test)]
mod unavailable_session_tests;

use crate::BackendError;
use crate::app_event::AppEventPublisher;
use crate::plugin::PluginApi;
use crate::session_setup::{
    SessionMcpError, SessionMcpHost, observe_session_mcp_health, resolve_session_mcp,
    resolve_session_mcp_revision,
};
use crate::task::resolve_workspace_cwd;
use ora_agent_runtime::{
    AgentAttach, AgentPluginAttachment, AgentRuntimeHost, AgentSessionMcpCapabilities,
    RuntimeError, RuntimeEvents, SessionMcpRevision, SessionMcpSnapshot, SessionSetup,
    SessionStore, WorkspaceDirectory,
};
use ora_application::{RepositoryError, SessionRepository};
use ora_contracts::{
    AppEvent, GetAgentRuntimeStatusRequest, GetAgentRuntimeStatusResponse,
    InstalledPluginContribution, ListAgentModelsRequest, ListAgentModelsResponse,
    ListInstalledPluginsRequest, StopPluginRequest,
};
use ora_db::{RepositoryPool, SqliteSessionRepository};
use ora_domain::{
    AgentRef, HistoryState, PluginId, Session, SessionId, SessionMcpSelection, SessionStatus,
    SessionTitle, WorkspaceId,
};
use ora_effect::ConsumerDeclaration;
use ora_history::HistoryLine;
use ora_plugin_lifecycle::{ConnectionError, PluginLifecycleError};
use ora_scheduler::Scheduler;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) use ora_agent_runtime::ReplacedAgentSessions;
pub(crate) use ora_agent_runtime::plugin_effect;

/// The runtime as Desktop composes it.
pub(crate) type AgentRuntimeManager = ora_agent_runtime::AgentRuntimeManager<DesktopRuntimeHost>;

/// Reports shared agent readiness and performs on-demand model discovery without exposing actors,
/// supervisor lifecycle control, or session creation to runtime-status consumers.
#[derive(Clone)]
pub struct AgentRuntime {
    inner: ora_agent_runtime::AgentRuntime<DesktopRuntimeHost>,
}

impl AgentRuntime {
    /// Captures the same manager used by sessions, plugins, workflow execution, and Effect.
    pub(crate) fn new(manager: Arc<AgentRuntimeManager>) -> Self {
        Self {
            inner: ora_agent_runtime::AgentRuntime::new(manager),
        }
    }

    /// Reports whether each application-scoped CLI runtime is ready, starting, or unavailable.
    pub fn status(
        &self,
        request: GetAgentRuntimeStatusRequest,
    ) -> Result<GetAgentRuntimeStatusResponse, BackendError> {
        Ok(self.inner.status(request)?)
    }

    /// Discovers one agent's models for a workspace without creating a session.
    pub async fn models(
        &self,
        request: ListAgentModelsRequest,
    ) -> Result<ListAgentModelsResponse, BackendError> {
        Ok(self.inner.models(request).await?)
    }
}

/// Names Desktop's implementation of every runtime host interface.
pub(crate) struct DesktopRuntimeHost;

impl AgentRuntimeHost for DesktopRuntimeHost {
    type Store = DesktopSessionStore;
    type Attach = PluginApi;
    type Setup = SessionMcpHost;
    type Events = AppEventPublisher;
    type Directory = DesktopWorkspaceDirectory;
}

/// Groups the fixed Desktop dependencies the agent runtime is constructed from.
pub(crate) struct AgentRuntimeSetup {
    /// Owns the processes behind plugin-provided agents and the set of installed packages.
    pub plugin_host: Arc<PluginApi>,
    pub pool: RepositoryPool,
    pub home_directory: PathBuf,
    pub relative_path_base: PathBuf,
    pub sessions_root: PathBuf,
    pub scheduler: Scheduler,
    pub app_events: AppEventPublisher,
    /// Host-owned MCP servers delivered beside the selected plugin servers.
    pub host_mcp_servers: Vec<agent_client_protocol_schema::v1::McpServer>,
}

/// Builds the Desktop runtime, reconciling stale rows and starting one supervisor per agent.
pub(crate) fn open_agent_runtime(
    setup: AgentRuntimeSetup,
) -> Result<AgentRuntimeManager, BackendError> {
    let AgentRuntimeSetup {
        plugin_host,
        pool,
        home_directory,
        relative_path_base,
        sessions_root,
        scheduler,
        app_events,
        host_mcp_servers,
    } = setup;
    Ok(AgentRuntimeManager::new(
        ora_agent_runtime::AgentRuntimeSetup {
            session_setup: SessionMcpHost::from_plugin_api(plugin_host.clone())
                .with_host_servers(host_mcp_servers),
            attach: plugin_host,
            store: DesktopSessionStore { pool: pool.clone() },
            directory: DesktopWorkspaceDirectory {
                pool,
                relative_path_base,
            },
            events: app_events,
            home_directory,
            sessions_root,
            scheduler,
        },
    )?)
}

/// Session rows in the Desktop SQLite database.
///
/// Holds the pool rather than a repository so each runtime call opens its repository the way
/// every other Desktop use case does.
#[derive(Clone)]
pub(crate) struct DesktopSessionStore {
    pool: RepositoryPool,
}

impl DesktopSessionStore {
    fn repository(&self) -> SqliteSessionRepository {
        SqliteSessionRepository::new(self.pool.clone())
    }
}

impl SessionStore for DesktopSessionStore {
    type Error = RepositoryError;

    fn create_session(&self, session: Session) -> Result<Session, RepositoryError> {
        self.repository().create_session(session)
    }

    fn find_session(&self, session_id: &SessionId) -> Result<Option<Session>, RepositoryError> {
        self.repository().find_session(session_id)
    }

    fn list_sessions(&self) -> Result<Vec<Session>, RepositoryError> {
        self.repository().list_sessions()
    }

    fn update_session_title(
        &self,
        session_id: &SessionId,
        title: &SessionTitle,
        now: i64,
    ) -> Result<Session, RepositoryError> {
        self.repository()
            .update_session_title(session_id, title, now)
    }

    fn update_session_status(
        &self,
        session_id: &SessionId,
        status: SessionStatus,
        now: i64,
    ) -> Result<Session, RepositoryError> {
        self.repository()
            .update_session_status(session_id, status, now)
    }

    fn update_session_binding(
        &self,
        session_id: &SessionId,
        agent_ref: AgentRef,
        agent_session_id: &str,
        now: i64,
    ) -> Result<Session, RepositoryError> {
        self.repository()
            .update_session_binding(session_id, agent_ref, agent_session_id, now)
    }

    fn update_session_history_state(
        &self,
        session_id: &SessionId,
        history_state: &HistoryState,
        now: i64,
    ) -> Result<Session, RepositoryError> {
        self.repository()
            .update_session_history_state(session_id, history_state, now)
    }

    fn soft_delete_session(
        &self,
        session_id: &SessionId,
        deleted_at: i64,
    ) -> Result<bool, RepositoryError> {
        self.repository()
            .soft_delete_session(session_id, deleted_at)
    }
}

impl AgentAttach for PluginApi {
    type StopError = PluginLifecycleError;

    /// Only agent-kind packages supply an agent; ids in the snapshot are canonical, so one that
    /// does not parse cannot occur and is skipped rather than failing the whole reconciliation.
    fn installed_agent_plugins(&self) -> Vec<PluginId> {
        self.list(ListInstalledPluginsRequest {})
            .plugins
            .into_iter()
            .filter(|plugin| {
                matches!(
                    plugin.contribution,
                    InstalledPluginContribution::Agent { .. }
                )
            })
            .filter_map(|plugin| PluginId::parse(&plugin.id).ok())
            .collect()
    }

    fn is_installed(&self, plugin_id: &PluginId) -> bool {
        self.list(ListInstalledPluginsRequest {})
            .plugins
            .iter()
            .any(|plugin| plugin.id == plugin_id.canonical())
    }

    async fn attach_agent(
        &self,
        plugin_id: &PluginId,
    ) -> Result<AgentPluginAttachment, ConnectionError> {
        let attachment = PluginApi::attach_agent(self, plugin_id).await?;
        Ok(AgentPluginAttachment {
            runtime: attachment.connection.runtime().process().clone(),
            notifications: attachment.notifications,
        })
    }

    async fn stop_plugin(&self, plugin_id: &PluginId) -> Result<(), PluginLifecycleError> {
        self.stop(StopPluginRequest {
            plugin_id: plugin_id.to_string(),
        })
        .await
        .map(|_stopped| ())
    }

    fn replace_agent_effect_declaration(
        &self,
        plugin_id: PluginId,
        declaration: Option<ConsumerDeclaration>,
    ) -> Result<(), RuntimeError> {
        PluginApi::replace_agent_effect_declaration(self, plugin_id, declaration)
            .map_err(RuntimeError::from)
    }
}

impl SessionSetup for SessionMcpHost {
    fn with_selection(&self, selection: SessionMcpSelection) -> Self {
        SessionMcpHost::with_selection(self, selection)
    }

    fn selection(&self) -> &SessionMcpSelection {
        &self.selection
    }

    fn host_mcp_servers(&self) -> Vec<agent_client_protocol_schema::v1::McpServer> {
        SessionMcpHost::host_mcp_servers(self)
    }

    fn desired_mcp_revision(&self) -> Result<SessionMcpRevision, RuntimeError> {
        resolve_session_mcp_revision(self, self, &self.selection).map_err(mcp_runtime_error)
    }

    fn resolve_mcp(
        &self,
        cwd: &Path,
        capabilities: AgentSessionMcpCapabilities,
    ) -> Result<SessionMcpSnapshot, RuntimeError> {
        resolve_session_mcp(self, self, cwd, capabilities, &self.selection)
            .map_err(mcp_runtime_error)
    }

    fn observe_mcp_health(&self, session_id: &SessionId, cwd: &Path) {
        observe_session_mcp_health(self, session_id, cwd);
    }

    fn plugin_id_for_agent(&self, agent_ref: &AgentRef) -> Option<PluginId> {
        SessionMcpHost::plugin_id_for_agent(self, agent_ref)
    }
}

/// Reports a Session MCP failure with the public error Desktop has always returned for it.
fn mcp_runtime_error(error: SessionMcpError) -> RuntimeError {
    RuntimeError::from(error.into_backend())
}

impl RuntimeEvents for AppEventPublisher {
    fn session_title_updated(&self, session_id: &SessionId) {
        self.try_publish(AppEvent::SessionTitleUpdated {
            session_id: session_id.to_string(),
        });
    }

    fn agent_models_invalidated(&self, agent_ref: &AgentRef) {
        self.try_publish(AppEvent::AgentModelsInvalidated {
            agent_ref: agent_ref.to_string(),
        });
    }

    /// Desktop clients read the history file itself, so there is no mirror to update.
    fn record_settled(&self, _session_id: &SessionId, _line: &HistoryLine) {}
}

/// Resolves a Workspace's local directory against the bootstrap path base.
pub(crate) struct DesktopWorkspaceDirectory {
    pool: RepositoryPool,
    relative_path_base: PathBuf,
}

impl WorkspaceDirectory for DesktopWorkspaceDirectory {
    fn workspace_cwd(&self, workspace_id: &WorkspaceId) -> Result<PathBuf, RuntimeError> {
        resolve_workspace_cwd(&self.pool, workspace_id, &self.relative_path_base)
            .map_err(RuntimeError::from)
    }
}

/// Owns one runtime operation stream and reports its failures as Desktop errors.
///
/// Dropping an incomplete stream cancels its operation exactly as the runtime stream does; the
/// wrapper only converts error types at the Desktop boundary.
pub struct SessionEventStream<Event> {
    inner: ora_agent_runtime::SessionEventStream<Event>,
}

impl<Event: Send + 'static> SessionEventStream<Event> {
    /// Uses the publisher's cleanup future to confirm its owned worker has exited.
    pub(crate) fn with_cleanup<F, Fut>(
        receiver: tokio::sync::mpsc::Receiver<Result<Event, RuntimeError>>,
        cleanup: F,
    ) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BackendError>> + Send + 'static,
    {
        Self {
            inner: ora_agent_runtime::SessionEventStream::with_cleanup(
                receiver,
                move || async move { cleanup().await.map_err(RuntimeError::from) },
            ),
        }
    }

    /// Runs workflow restoration after the operation owner has settled, retaining the first failure.
    pub(crate) fn attach_cleanup<F, Fut>(self, cleanup: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), BackendError>> + Send + 'static,
    {
        Self {
            inner: self
                .inner
                .attach_cleanup(move || async move { cleanup().await.map_err(RuntimeError::from) }),
        }
    }

    /// Receives the next ordered backend event.
    pub async fn recv(&mut self) -> Option<Result<Event, BackendError>> {
        self.inner
            .recv()
            .await
            .map(|item| item.map_err(BackendError::from))
    }

    /// Returns a buffered item without waiting during HTTP shutdown.
    pub fn try_recv(&mut self) -> Option<Result<Event, BackendError>> {
        self.inner
            .try_recv()
            .map(|item| item.map_err(BackendError::from))
    }

    /// Hands the turn owner the same report the runtime writes, so cleanup can read it later.
    pub(crate) fn end_report(&self) -> Option<ora_agent_runtime::PromptEndReport> {
        self.inner.end_report()
    }

    /// Requests cancellation once and waits for owner cleanup followed by workflow restoration.
    pub async fn cancel_and_wait(&mut self) -> Result<(), BackendError> {
        self.inner
            .cancel_and_wait()
            .await
            .map_err(BackendError::from)
    }
}

impl<Event> From<ora_agent_runtime::SessionEventStream<Event>> for SessionEventStream<Event> {
    fn from(inner: ora_agent_runtime::SessionEventStream<Event>) -> Self {
        Self { inner }
    }
}
