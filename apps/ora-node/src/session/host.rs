//! The Node's implementations of the agent runtime's host interfaces, scoped to one session
//! execution.
//!
//! Each execution composes its own set, so "only this plugin" and "only this Git identity" hold by
//! construction: the attach knows exactly one plugin, the lifecycle behind it launches with exactly
//! one identity, and the mirror writes to exactly one execution.

use super::ports::SessionLedger;
use super::thread::thread_event;
use ora_agent_runtime::{
    AgentAttach, AgentPluginAttachment, AgentRuntimeHost, MemorySessionStore, NoSessionMcp,
    RuntimeError, RuntimeEvents, WorkspaceDirectory,
};
use ora_contracts::StopPluginRequest;
use ora_domain::{AgentRef, PluginId, SessionId, WorkspaceId};
use ora_effect::ConsumerDeclaration;
use ora_history::HistoryLine;
use ora_logging::ora_warn;
use ora_node_protocol::{ExecutionId, GitIdentity, TurnId};
use ora_plugin_lifecycle::{
    ChildProcessEnvironmentProvider, ConnectionError, DenoPluginRuntimeLauncher, GenerationTaps,
    PluginLifecycle, PluginLifecycleError, PluginStatusPublisher,
};
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::Notify;

/// How long attaching waits for the plugin launch to settle.
///
/// Longer than the runtime's ready timeout, so a slow handshake is reported by the launch itself,
/// with its reason, rather than as an opaque attach timeout.
const AGENT_ATTACH_WAIT: Duration = Duration::from_secs(15);

/// The plugin lifecycle one session execution owns.
pub(super) type SessionLifecycle = PluginLifecycle<
    DenoPluginRuntimeLauncher<GitIdentityEnvironment>,
    IgnoredStatus,
    GenerationTaps,
>;

/// Names the Node's host composition for one session execution over ledger `L`.
pub(super) struct NodeRuntimeHost<L>(PhantomData<L>);

impl<L: SessionLedger> AgentRuntimeHost for NodeRuntimeHost<L> {
    /// A Node never resumes a session after restarting, so session rows need not outlive it.
    type Store = MemorySessionStore;
    type Attach = SessionPlugin;
    /// Sessions receive no MCP servers until their configuration and secrets have a delivery path.
    type Setup = NoSessionMcp;
    type Events = ThreadMirror<L>;
    type Directory = CheckoutDirectory;
}

/// Exports one commit identity to the agent plugin and to everything it runs.
///
/// Set on the plugin process, so a process the plugin spawns directly inherits it, and on every
/// process the host spawns for the plugin, which inherits the host's environment instead. Nothing
/// is written to a Git configuration.
#[derive(Clone)]
pub(super) struct GitIdentityEnvironment {
    variables: BTreeMap<String, String>,
}

impl GitIdentityEnvironment {
    /// Exports host-owned runtime variables with the authoritative commit identity.
    pub(super) fn new(identity: &GitIdentity, runtime: BTreeMap<String, String>) -> Self {
        let mut variables = runtime;
        variables.extend(
            [
                ("GIT_AUTHOR_NAME", &identity.name),
                ("GIT_AUTHOR_EMAIL", &identity.email),
                ("GIT_COMMITTER_NAME", &identity.name),
                ("GIT_COMMITTER_EMAIL", &identity.email),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.clone())),
        );
        Self { variables }
    }
}

impl ChildProcessEnvironmentProvider for GitIdentityEnvironment {
    /// The lifecycle behind it runs only this session's plugin in this session's checkout, so
    /// every host-spawned process is one this identity belongs to.
    fn environment(
        &self,
        _plugin_id: &str,
        _workspace_root: &Path,
    ) -> Result<BTreeMap<String, String>, String> {
        Ok(self.variables.clone())
    }

    fn plugin_environment(&self, _plugin_id: &str) -> BTreeMap<String, String> {
        self.variables.clone()
    }
}

/// Plugin status changes have no audience on a Node: the session reports through its own result.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct IgnoredStatus;

impl PluginStatusPublisher for IgnoredStatus {
    fn publish_status_changed(&self, _plugin_id: &PluginId) {}
}

/// Reaches the one agent plugin a session execution runs.
pub(super) struct SessionPlugin {
    pub(super) lifecycle: SessionLifecycle,
    pub(super) taps: GenerationTaps,
    pub(super) plugin_id: PluginId,
}

impl AgentAttach for SessionPlugin {
    type StopError = PluginLifecycleError;

    /// The session verified this plugin's version before composing the runtime, so it is the
    /// whole answer; any other installed plugin stays invisible and never starts.
    fn installed_agent_plugins(&self) -> Vec<PluginId> {
        vec![self.plugin_id.clone()]
    }

    fn is_installed(&self, plugin_id: &PluginId) -> bool {
        *plugin_id == self.plugin_id
    }

    /// Pins one running generation, then taps exactly that generation's notifications.
    async fn attach_agent(
        &self,
        plugin_id: &PluginId,
    ) -> Result<AgentPluginAttachment, ConnectionError> {
        if *plugin_id != self.plugin_id {
            return Err(ConnectionError::NotFound);
        }
        let connection = self
            .lifecycle
            .ensure_running(plugin_id, AGENT_ATTACH_WAIT)
            .await?;
        let notifications = self.taps.tap(plugin_id, connection.key());
        Ok(AgentPluginAttachment {
            runtime: connection.runtime().process().clone(),
            notifications,
        })
    }

    async fn stop_plugin(&self, plugin_id: &PluginId) -> Result<(), PluginLifecycleError> {
        self.lifecycle
            .stop_plugin(StopPluginRequest {
                plugin_id: plugin_id.to_string(),
            })
            .await
            .map(drop)
    }

    /// A Node projects no Skills into the checkout, so there is no Effect consumer to record.
    fn replace_agent_effect_declaration(
        &self,
        _plugin_id: PluginId,
        _declaration: Option<ConsumerDeclaration>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
}

/// Every Workspace of a session execution is its checkout.
pub(super) struct CheckoutDirectory(pub(super) PathBuf);

impl WorkspaceDirectory for CheckoutDirectory {
    fn workspace_cwd(&self, _workspace_id: &WorkspaceId) -> Result<PathBuf, RuntimeError> {
        Ok(self.0.clone())
    }
}

/// Mirrors each settled history line of one execution into the ledger as a Thread event.
///
/// Lines arrive after the history file holds them, so the Thread never has a record the file
/// lacks. After the first failed append the mirror stops for good and wakes the session to end:
/// appending later lines would leave a hole in the Thread that nothing could explain.
#[derive(Clone)]
pub(super) struct ThreadMirror<L> {
    ledger: L,
    execution: ExecutionId,
    turn: Arc<Mutex<Option<TurnId>>>,
    broken: Arc<AtomicBool>,
    wake: Arc<Notify>,
}

impl<L: SessionLedger> ThreadMirror<L> {
    pub(super) fn new(ledger: L, execution: ExecutionId, wake: Arc<Notify>) -> Self {
        Self {
            ledger,
            execution,
            turn: Arc::new(Mutex::new(None)),
            broken: Arc::new(AtomicBool::new(false)),
            wake,
        }
    }

    /// Attributes every line settled from now on to `turn_id`, or to no turn.
    pub(super) fn set_turn(&self, turn_id: Option<TurnId>) {
        *self.turn.lock().unwrap_or_else(PoisonError::into_inner) = turn_id;
    }

    /// Reports whether a line failed to reach the ledger.
    pub(super) fn is_broken(&self) -> bool {
        self.broken.load(Ordering::Acquire)
    }

    /// Stops mirroring after a failure and wakes the session so it can end.
    fn break_off(&self, error: &dyn std::fmt::Display) {
        ora_warn!(
            execution_id = %self.execution,
            error = %error,
            "session record could not become a Thread event",
        );
        self.broken.store(true, Ordering::Release);
        self.wake.notify_one();
    }
}

impl<L: SessionLedger> RuntimeEvents for ThreadMirror<L> {
    /// A Thread has no title of its own; Cloud names the run.
    fn session_title_updated(&self, _session_id: &SessionId) {}

    /// A session execution never offers a model choice after it starts.
    fn agent_models_invalidated(&self, _agent_ref: &AgentRef) {}

    fn record_settled(&self, _session_id: &SessionId, line: &HistoryLine) {
        if self.is_broken() {
            return;
        }
        let turn_id = self
            .turn
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let appended = thread_event(line, turn_id)
            .map_err(|error| error.to_string())
            .and_then(|event| {
                self.ledger
                    .append_thread_event(&self.execution, event)
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = appended {
            self.break_off(&error);
        }
    }
}
