//! In-memory host for runtime tests.
//!
//! Each fake keeps the observable contract of the interface it stands in for — a store that
//! hides soft-deleted rows, an attach that reports only what is installed, a setup that resolves
//! an empty MCP set — so a test exercising the runtime through it fails for the runtime's reasons
//! rather than the fake's.

use crate::host::{
    AgentAttach, AgentPluginAttachment, AgentRuntimeHost, RuntimeEvents, WorkspaceDirectory,
};
use crate::{
    AgentRuntimeManager, AgentRuntimeSetup, MemorySessionStore, NoSessionMcp, NoSkillCatalog,
    RuntimeError,
};
use ora_domain::{AgentRef, PluginId, SessionId, WorkspaceId};
use ora_history::HistoryLine;
use ora_plugin_lifecycle::{ConnectionError, RedactionRegistry};
use ora_scheduler::Scheduler;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use thiserror::Error;

/// Composes the in-memory fakes into one runtime host.
pub(crate) struct TestHost;

impl AgentRuntimeHost for TestHost {
    type Store = MemorySessionStore;
    type Attach = InstalledAgents;
    type Setup = NoSessionMcp;
    type Skills = NoSkillCatalog;
    type Events = RecordedEvents;
    type Directory = FixedDirectory;
}

/// Reports a fixed set of installed agent plugins, none of which ever starts.
///
/// Attaching always finds no process, which the supervisor treats as an agent that is not
/// installed yet and keeps retrying quietly — the same state an installed but disabled package
/// leaves Desktop in.
#[derive(Default)]
pub(crate) struct InstalledAgents {
    plugins: Mutex<Vec<PluginId>>,
}

impl InstalledAgents {
    /// Reports exactly `plugins` as installed agent packages.
    pub(crate) fn new(plugins: Vec<PluginId>) -> Self {
        Self {
            plugins: Mutex::new(plugins),
        }
    }

    /// Adds one agent package, as an install while the host is running would.
    pub(crate) fn install(&self, plugin_id: PluginId) {
        self.plugins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(plugin_id);
    }
}

/// Stopping a plugin that never started cannot fail.
#[derive(Debug, Error)]
#[error("unreachable")]
pub(crate) enum NeverFails {}

impl AgentAttach for InstalledAgents {
    type StopError = NeverFails;

    fn installed_agent_plugins(&self) -> Vec<PluginId> {
        self.plugins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn is_installed(&self, plugin_id: &PluginId) -> bool {
        self.plugins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(plugin_id)
    }

    async fn attach_agent(
        &self,
        _plugin_id: &PluginId,
    ) -> Result<AgentPluginAttachment, ConnectionError> {
        Err(ConnectionError::NoProcess)
    }

    async fn stop_plugin(&self, _plugin_id: &PluginId) -> Result<(), NeverFails> {
        Ok(())
    }
}

/// Keeps every settled history line in order for assertions.
#[derive(Clone, Default)]
pub(crate) struct RecordedEvents {
    settled: Arc<Mutex<Vec<(SessionId, HistoryLine)>>>,
    options_changed: Arc<Mutex<Vec<SessionId>>>,
}

impl RecordedEvents {
    /// Returns every settled history line in the order the runtime reported it.
    pub(crate) fn settled(&self) -> Vec<(SessionId, HistoryLine)> {
        self.settled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Returns every session the runtime said may have other options, in announcement order.
    pub(crate) fn options_changed(&self) -> Vec<SessionId> {
        self.options_changed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RuntimeEvents for RecordedEvents {
    fn session_title_updated(&self, _session_id: &SessionId) {}

    fn session_options_changed(&self, session_id: &SessionId) {
        self.options_changed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(session_id.clone());
    }

    fn agent_models_invalidated(&self, _agent_ref: &AgentRef) {}

    fn record_settled(&self, session_id: &SessionId, line: &HistoryLine) {
        self.settled
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((session_id.clone(), line.clone()));
    }
}

/// Resolves every Workspace to one existing directory.
pub(crate) struct FixedDirectory {
    root: PathBuf,
}

impl WorkspaceDirectory for FixedDirectory {
    fn workspace_cwd(&self, _workspace_id: &WorkspaceId) -> Result<PathBuf, RuntimeError> {
        Ok(self.root.clone())
    }
}

/// Everything one test manager was built over, kept so the test can inspect it afterwards.
pub(crate) struct TestRuntime {
    pub manager: AgentRuntimeManager<TestHost>,
}

/// Builds a manager rooted at `root` with `installed` agent plugins over `store`.
pub(crate) fn test_runtime(
    root: &Path,
    installed: Vec<PluginId>,
    store: MemorySessionStore,
    scheduler: Scheduler,
) -> TestRuntime {
    let manager = AgentRuntimeManager::new(AgentRuntimeSetup::<TestHost> {
        attach: Arc::new(InstalledAgents::new(installed)),
        redactions: RedactionRegistry::default(),
        store,
        directory: FixedDirectory {
            root: root.to_path_buf(),
        },
        session_setup: NoSessionMcp::default(),
        skill_catalog: NoSkillCatalog,
        events: RecordedEvents::default(),
        home_directory: root.to_path_buf(),
        sessions_root: root.join("sessions"),
        scheduler,
    })
    .expect("build agent runtime manager");
    TestRuntime { manager }
}
