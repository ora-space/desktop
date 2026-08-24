//! Traits and value types through which the lifecycle talks to processes, the host, and tests.
//!
//! Everything here is a seam: the lifecycle never touches a real process, event hub, or desktop
//! window directly, so each of those can be replaced by a test double or a different runtime
//! composition without changing orchestration code.

use crate::permissions::DenoPermission;
use ora_domain::PluginId;
use ora_plugin_runtime::{PluginNotification, PluginRegistration};
use serde_json::Value;
use std::future::Future;
use std::path::PathBuf;
use thiserror::Error;
use tokio::sync::mpsc;

/// Describes one concrete process launch after package discovery has resolved its entrypoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLaunchRequest {
    pub plugin_id: PluginId,
    pub deno_path: PathBuf,
    pub entrypoint: PathBuf,
    /// Package root, used as the process working directory so relative imports resolve locally.
    pub package_root: PathBuf,
    pub permissions: Vec<DenoPermission>,
    /// The plugin's private data directory, already created; the launcher binds the
    /// `ora/storage/*` handler to it so the process can only ever reach its own data.
    pub data_dir: PathBuf,
}

/// Preserves the reason a plugin process could not start or stopped unexpectedly.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{reason}")]
pub struct PluginRuntimeFailure {
    reason: String,
}

impl PluginRuntimeFailure {
    /// Creates one failure reason suitable for the public failed lifecycle state.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// Returns the stable human-readable reason retained by lifecycle state.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// Distinguishes an intentional process exit from an unexpected runtime failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRuntimeExit {
    Stopped,
    Failed(PluginRuntimeFailure),
}

/// Reports why one host-originated call into a plugin could not complete.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PluginCallError {
    #[error("plugin is not serving calls")]
    Unavailable,
    #[error("plugin did not register the requested method")]
    MethodNotRegistered,
    #[error("plugin call timed out")]
    Timeout,
    #[error("plugin method failed with code {code}: {message}")]
    Remote { code: i64, message: String },
    #[error("plugin transport failed: {0}")]
    Transport(String),
}

/// Owns one launched plugin process: explicit stop, exit observation, and the call data plane.
pub trait PluginRuntime: Clone + Send + Sync + 'static {
    /// Stops the complete plugin process tree and resolves only after it has exited.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send;

    /// Waits until the process exits and preserves whether shutdown was intentional.
    fn wait_for_exit(&self) -> impl Future<Output = PluginRuntimeExit> + Send + 'static;

    /// Returns the capability declaration the plugin published during its handshake.
    ///
    /// The registration is immutable once the plugin is ready, which is why this is a plain
    /// accessor: contract validation runs exactly once, right after launch.
    fn registration(&self) -> PluginRegistration;

    /// Invokes one registered method and returns its JSON result.
    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, PluginCallError>> + Send;
}

/// Pairs a ready runtime with the stream of notifications the plugin sends on its own.
///
/// The receiver is handed over rather than wrapped because the lifecycle owns exactly one pump
/// per launch; keeping the raw channel lets that pump observe closure, which is how reader loss
/// becomes visible without the process itself having exited.
pub struct LaunchedRuntime<Runtime> {
    pub runtime: Runtime,
    pub notifications: mpsc::UnboundedReceiver<PluginNotification>,
}

/// Launches plugin runtimes while allowing tests to replace the external process boundary.
pub trait PluginRuntimeLauncher: Clone + Send + Sync + 'static {
    type Runtime: PluginRuntime;

    /// Starts one resolved plugin entrypoint and returns after runtime readiness is established.
    fn launch(
        &self,
        request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<LaunchedRuntime<Self::Runtime>, PluginRuntimeFailure>> + Send;
}

/// Publishes cache invalidations after observable plugin lifecycle transitions.
pub trait PluginStatusPublisher: Clone + Send + Sync + 'static {
    /// Announces that consumers should query the installed-plugin snapshot again.
    fn publish_status_changed(&self, plugin_id: &PluginId);
}

/// Carries one plugin-originated notification together with the process that produced it.
///
/// `generation` lets consumers drop frames from a process that has since been replaced: a
/// notification is only meaningful relative to the connection the consumer is talking to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundNotification {
    pub plugin_id: PluginId,
    pub generation: crate::connection::PluginGenerationKey,
    pub method: String,
    pub params: Value,
}

/// Receives every whitelisted notification a running plugin emits.
///
/// Implementations must not block: the pump that calls them is the only consumer of the process
/// stream, and a slow sink would stall delivery for that plugin. Buffering belongs to the sink.
pub trait PluginNotificationSink: Clone + Send + Sync + 'static {
    /// Delivers one notification in the order the plugin sent it.
    fn on_notification(&self, notification: InboundNotification);
}
