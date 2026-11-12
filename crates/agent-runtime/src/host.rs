//! Host interfaces the agent runtime is composed from.
//!
//! The runtime owns ACP session semantics; everything that depends on where it runs — how session
//! rows are stored, how an agent plugin process is reached, which MCP servers a session receives,
//! who hears about lifecycle events, and which directory a Workspace maps to — is supplied by the
//! host through these traits. Desktop implements them over SQLite and its plugin API; a sandbox
//! Node implements them over its own ledger and plugin lifecycle.

use agent_client_protocol_schema::v1::McpServer;
use crate::error::RuntimeError;
use crate::session_setup::{AgentSessionMcpCapabilities, SessionMcpRevision, SessionMcpSnapshot};
use ora_domain::{
    AgentRef, HistoryState, PluginId, Session, SessionId, SessionMcpSelection, SessionStatus,
    SessionTitle, WorkspaceId,
};
use ora_effect::ConsumerDeclaration;
use ora_history::HistoryLine;
use ora_plugin_lifecycle::{ConnectionError, InboundNotification};
use ora_plugin_runtime::PluginRuntime;
use std::error::Error;
use std::future::Future;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Bundles one host's implementations of every runtime interface.
///
/// Each capability is an associated type so the runtime is monomorphized per host instead of
/// dispatching through trait objects, and so a host names its whole composition in one place.
pub trait AgentRuntimeHost: Send + Sync + 'static {
    type Store: SessionStore;
    type Attach: AgentAttach;
    type Setup: SessionSetup;
    type Events: RuntimeEvents;
    type Directory: WorkspaceDirectory;
}

/// Durable storage for Ora Session rows.
///
/// Implementations are cheap handles: the runtime clones one per actor and per operation. Every
/// update returns the complete current row, because the actor replaces its in-memory copy with it
/// rather than patching fields it assumes the store did not touch.
pub trait SessionStore: Clone + Send + Sync + 'static {
    type Error: Error + Send + Sync + 'static;

    /// Persists a newly created session and returns the stored snapshot.
    fn create_session(&self, session: Session) -> Result<Session, Self::Error>;

    /// Loads one visible session by identifier.
    fn find_session(&self, session_id: &SessionId) -> Result<Option<Session>, Self::Error>;

    /// Lists every visible session in storage order.
    fn list_sessions(&self) -> Result<Vec<Session>, Self::Error>;

    /// Updates only the durable display title and returns the complete current row.
    fn update_session_title(
        &self,
        session_id: &SessionId,
        title: &SessionTitle,
        now: i64,
    ) -> Result<Session, Self::Error>;

    /// Updates only lifecycle status and returns the complete current row.
    fn update_session_status(
        &self,
        session_id: &SessionId,
        status: SessionStatus,
        now: i64,
    ) -> Result<Session, Self::Error>;

    /// Updates only provider binding and returns the complete current row.
    fn update_session_binding(
        &self,
        session_id: &SessionId,
        agent_ref: AgentRef,
        agent_session_id: &str,
        now: i64,
    ) -> Result<Session, Self::Error>;

    /// Updates only history state and returns the complete current row.
    fn update_session_history_state(
        &self,
        session_id: &SessionId,
        history_state: &HistoryState,
        now: i64,
    ) -> Result<Session, Self::Error>;

    /// Marks a session deleted and returns whether a visible session was affected.
    fn soft_delete_session(
        &self,
        session_id: &SessionId,
        deleted_at: i64,
    ) -> Result<bool, Self::Error>;
}

/// A running agent plugin process together with a lossless stream of what it emits.
///
/// The notification stream must cover exactly the process generation `runtime` addresses, so a
/// restarted plugin can never leak frames into a connection that belonged to its predecessor.
pub struct AgentPluginAttachment {
    pub runtime: PluginRuntime,
    pub notifications: mpsc::UnboundedReceiver<InboundNotification>,
}

/// Reaches the agent plugin processes the runtime speaks ACP through.
///
/// The host owns plugin processes; the runtime only attaches to one, speaks the agent contract
/// over it, and asks the host to stop it when a connection generation ends. Which plugins exist is
/// likewise the host's answer, so a host that serves one agent reports exactly that one.
pub trait AgentAttach: Send + Sync + 'static {
    /// Why the host could not stop a plugin; the runtime only logs it.
    type StopError: Error + Send + Sync + 'static;

    /// Supplies a live, sensitive startup context only for this host's frozen execution.
    fn model_context(&self) -> Option<ora_plugin_protocol::AgentModelContext> {
        None
    }

    /// Lists the canonical ids of installed plugins that contribute an agent.
    fn installed_agent_plugins(&self) -> Vec<PluginId>;

    /// Reports whether any installed plugin, of any kind, carries this canonical id.
    fn is_installed(&self, plugin_id: &PluginId) -> bool;

    /// Returns a connection to a running plugin, starting the installed plugin when it is stopped.
    fn attach_agent(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<AgentPluginAttachment, ConnectionError>> + Send;

    /// Stops one plugin process after its agent generation failed or shut down.
    fn stop_plugin(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<(), Self::StopError>> + Send;

    /// Records the Effect consumer declaration an agent plugin registered, or clears it.
    ///
    /// A failure is terminal for the connection: the plugin registered something the host cannot
    /// honour, and retrying would register the same thing again.
    fn replace_agent_effect_declaration(
        &self,
        plugin_id: PluginId,
        declaration: Option<ConsumerDeclaration>,
    ) -> Result<(), RuntimeError>;
}

/// Resolves the MCP servers every ACP `session/new` and `session/load` sends.
///
/// A value is a session-scoped view: `with_selection` narrows the host's shared source to one
/// session's stored selection without mutating it. Implementations must be side-effect free apart
/// from `observe_mcp_health`, because the runtime re-reads the Desired revision whenever it is
/// woken and compares revisions in memory.
pub trait SessionSetup: Clone + Send + Sync + 'static {
    /// Creates a session-local view without mutating the shared source.
    fn with_selection(&self, selection: SessionMcpSelection) -> Self;

    /// The selection this view resolves.
    fn selection(&self) -> &SessionMcpSelection;

    /// Host-owned servers delivered beside the selected plugin MCP servers.
    ///
    /// These are not part of the session selection or its revision; they are a runtime capability
    /// the host injects into every ACP setup for this session view.
    fn host_mcp_servers(&self) -> Vec<McpServer> {
        Vec::new()
    }

    /// Builds the secret-free Desired revision without producing ACP payloads.
    fn desired_mcp_revision(&self) -> Result<SessionMcpRevision, RuntimeError>;

    /// Resolves one complete Snapshot for a single ACP setup or refresh.
    fn resolve_mcp(
        &self,
        cwd: &Path,
        capabilities: AgentSessionMcpCapabilities,
    ) -> Result<SessionMcpSnapshot, RuntimeError>;

    /// Schedules the host's observation of the servers just delivered to one session.
    fn observe_mcp_health(&self, session_id: &SessionId, cwd: &Path);

    /// Translates a Session's agent identity into the plugin that owns its barrier.
    fn plugin_id_for_agent(&self, agent_ref: &AgentRef) -> Option<PluginId>;
}

/// Receives the lifecycle notifications a host surfaces to its clients.
///
/// Delivery is best effort and must not block: the runtime publishes from inside session actors.
pub trait RuntimeEvents: Clone + Send + Sync + 'static {
    /// Opts a host into bounded-rate pending snapshots, independent of durable history callbacks.
    fn wants_record_previews(&self) -> bool {
        false
    }

    /// Replaces the host's still-open records. This callback is optional, lossy and memory-only.
    fn record_preview(&self, _session_id: &SessionId, _records: &[ora_history::AssembledRecord]) {}
    /// A session's persisted title changed.
    fn session_title_updated(&self, session_id: &SessionId);

    /// A replaced agent process may expose a different model catalog.
    fn agent_models_invalidated(&self, agent_ref: &AgentRef);

    /// One line was just appended to a session's history file.
    ///
    /// Called synchronously from the session's actor after the line is in the file and before the
    /// next one is written, so a host that mirrors the conversation elsewhere sees every line
    /// exactly once, in file order, never one the file lacks, and after a crash is behind the file
    /// by at most one line. A host may do a short durable write here; anything slower stalls the
    /// session. A host with no mirror ignores it.
    fn record_settled(&self, session_id: &SessionId, line: &HistoryLine);
}

/// Maps a Workspace onto the directory its sessions run in.
pub trait WorkspaceDirectory: Send + Sync + 'static {
    /// Resolves the absolute `cwd` for sessions of one Workspace.
    fn workspace_cwd(&self, workspace_id: &WorkspaceId) -> Result<PathBuf, RuntimeError>;
}
