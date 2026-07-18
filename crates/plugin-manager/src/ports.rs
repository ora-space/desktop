use crate::{AgentInvocationHandle, PluginError, PluginLaunchGrant};
use ora_plugin_protocol::{
    AgentProviderId, AgentRequest, ContentDigest, ContentOwnerId, JsonSafeU64, PluginId,
    PluginKind, PluginVersion,
};
use std::future::Future;
use std::path::PathBuf;

/// A fresh management proof consumed by exactly one runtime start generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedLaunchDescriptor {
    pub plugin_id: PluginId,
    pub plugin_version: PluginVersion,
    pub kind: PluginKind,
    pub content_digest: ContentDigest,
    pub content_owner: ContentOwnerId,
    pub extension_path: PathBuf,
    pub entry_path: PathBuf,
    pub storage_path: PathBuf,
    pub declared_agents: Vec<AgentProviderId>,
    pub enablement_epoch: JsonSafeU64,
    pub registry_revision: JsonSafeU64,
    pub launch_grant: Option<PluginLaunchGrant>,
}

/// Rebuilds launch admission from current state, catalog, registry, and filesystem facts.
pub trait RuntimeAdmissionProvider {
    /// Returns a fresh descriptor or fails closed before any process is spawned.
    fn admit(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<ValidatedLaunchDescriptor, PluginError>> + Send;

    /// Rechecks the epoch/revision barrier after activate succeeds and before Running admission.
    fn recheck_after_activate(
        &self,
        descriptor: &ValidatedLaunchDescriptor,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;
}

/// The closed reasons management or backend lifecycle can stop a generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    ManualStop,
    Disable,
    Uninstall,
    Shutdown,
    GrantChanged,
}

/// Critical generation events used by management crash policy and cleanup state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRuntimeEvent {
    Started {
        plugin_id: PluginId,
        content_owner: ContentOwnerId,
        generation: JsonSafeU64,
        sequence: JsonSafeU64,
    },
    Stopped {
        plugin_id: PluginId,
        content_owner: ContentOwnerId,
        generation: JsonSafeU64,
        sequence: JsonSafeU64,
    },
    Crashed {
        plugin_id: PluginId,
        content_owner: ContentOwnerId,
        generation: JsonSafeU64,
        sequence: JsonSafeU64,
        exit_code: Option<i32>,
    },
    TreeReaped {
        plugin_id: PluginId,
        content_owner: ContentOwnerId,
        generation: JsonSafeU64,
        sequence: JsonSafeU64,
    },
}

/// Persists critical lifecycle events before runtime opens another generation.
pub trait PluginRuntimeEventSink {
    /// Accepts an ordered event; failure requires runtime to close admission and clean the tree.
    fn record(
        &self,
        event: PluginRuntimeEvent,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;
}

/// Closes runtime admission and proves complete tree cleanup for management mutations.
pub trait PluginRuntimeControl: Clone + Send + Sync + 'static {
    /// Reopens the hub-level gate after a durable enable or crash-loop reset succeeds.
    fn open_admission(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;

    /// Prevents new starts/invocations before a durable disable or removal mutation.
    fn close_admission(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;

    /// Stops a generation and returns only after direct process and tree-empty settlement.
    fn stop_and_reap(
        &self,
        plugin_id: &PluginId,
        reason: StopReason,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;

    /// Clears the transient supervisor crash-loop gate after durable management authorization.
    fn reset_crash_loop(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;
}

/// A management-test runtime boundary that performs no spawn and records no transient state.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopPluginRuntimeControl;

impl PluginRuntimeControl for NoopPluginRuntimeControl {
    async fn open_admission(&self, _plugin_id: &PluginId) -> Result<(), PluginError> {
        Ok(())
    }

    async fn close_admission(&self, _plugin_id: &PluginId) -> Result<(), PluginError> {
        Ok(())
    }

    async fn stop_and_reap(
        &self,
        _plugin_id: &PluginId,
        _reason: StopReason,
    ) -> Result<(), PluginError> {
        Ok(())
    }

    async fn reset_crash_loop(&self, _plugin_id: &PluginId) -> Result<(), PluginError> {
        Ok(())
    }
}

/// Routes typed Agent invocations to the lazy single-generation runtime for one plugin id.
pub trait PluginRuntimeInvocation {
    fn start(&self, plugin_id: &PluginId) -> impl Future<Output = Result<(), PluginError>> + Send;

    fn stop(
        &self,
        plugin_id: &PluginId,
        reason: StopReason,
    ) -> impl Future<Output = Result<(), PluginError>> + Send;

    fn invoke(
        &self,
        plugin_id: &PluginId,
        request: AgentRequest,
    ) -> impl Future<Output = Result<AgentInvocationHandle, PluginError>> + Send;

    /// Closes every hub admission and proves all known process trees are empty.
    fn shutdown_all(&self) -> impl Future<Output = Result<(), PluginError>> + Send;
}
