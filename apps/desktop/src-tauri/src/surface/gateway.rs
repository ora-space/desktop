//! The plugin data-plane port the surface layer depends on.
//!
//! `ora-backend` exposes a concrete `PluginGateway`; this trait mirrors the subset the surface
//! host uses so the service can be driven by a fake in tests without a database or a Deno
//! process. Production binds it to `Arc<ora_backend::PluginGateway>`.

use ora_backend::{GatewayError, PluginGateway};
use ora_domain::PluginId;
use ora_plugin_lifecycle::{
    ConnectionError, PluginCallError, PluginGenerationKey, PluginGenerationLease, PluginRuntime,
};
use ora_surface::SurfaceDefinition;
use serde_json::Value;
use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

/// A live lease on one plugin process generation.
///
/// Implementations forward JSON-RPC calls to the process; the workbench bridge needs the
/// generation key (for the envelope and logging), the methods the generation registered (to
/// intersect with the manifest allowlist), and the request call shape.
pub trait SurfaceConnection: Clone + Send + Sync + 'static {
    /// Returns the process generation this lease talks to.
    fn key(&self) -> PluginGenerationKey;

    /// Returns the methods this generation registered at its handshake.
    fn registered_methods(&self) -> HashSet<String>;

    /// Sends a request and waits for the plugin's result.
    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, PluginCallError>> + Send;
}

impl<R: PluginRuntime> SurfaceConnection for PluginGenerationLease<R> {
    fn key(&self) -> PluginGenerationKey {
        PluginGenerationLease::key(self)
    }

    fn registered_methods(&self) -> HashSet<String> {
        PluginGenerationLease::registered_methods(self)
    }

    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, PluginCallError>> + Send {
        PluginGenerationLease::invoke(self, method, params)
    }
}

/// Why the gateway could not serve a request.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum GatewayFailure {
    #[error(transparent)]
    Connection(ConnectionError),
    #[error("{0}")]
    Other(String),
}

impl From<GatewayError> for GatewayFailure {
    fn from(error: GatewayError) -> Self {
        match error {
            GatewayError::Connection(connection) => Self::Connection(connection),
            GatewayError::DataDirectory(_) | GatewayError::Lifecycle(_) => {
                Self::Other(error.to_string())
            }
        }
    }
}

/// Plugin lookup, data directory, and process access as seen by the surface host.
///
/// Implementations must be cheap to clone-by-reference (the service shares one instance with
/// spawned tasks) and must never block the caller on process startup except in
/// `ensure_running`.
pub trait SurfacePluginGateway: Clone + Send + Sync + 'static {
    type Connection: SurfaceConnection;

    /// Returns the surface an installed plugin contributes (`None` when it is not installed or
    /// contributes none, such as an agent), enabled or not.
    fn surface_definition(&self, plugin_id: &PluginId) -> Option<SurfaceDefinition>;

    /// Reports whether the plugin is installed and enabled.
    fn plugin_enabled(&self, plugin_id: &PluginId) -> bool;

    /// Creates and returns `<data-dir>/plugins/data/<namespace>/<name>`.
    fn data_directory(&self, plugin_id: &PluginId) -> Result<PathBuf, GatewayFailure>;

    /// Starts the plugin if needed and waits up to `wait` for a running connection.
    fn ensure_running(
        &self,
        plugin_id: &PluginId,
        wait: Duration,
    ) -> impl Future<Output = Result<Self::Connection, GatewayFailure>> + Send;

    /// Stops the plugin process; the caller has already verified that no instance is left.
    fn stop_if_idle(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<(), GatewayFailure>> + Send;
}

impl SurfacePluginGateway for Arc<PluginGateway> {
    type Connection = PluginGenerationLease<ora_plugin_lifecycle::DenoPluginRuntime>;

    fn surface_definition(&self, plugin_id: &PluginId) -> Option<SurfaceDefinition> {
        PluginGateway::installed_plugin(self, plugin_id)
            .and_then(|plugin| SurfaceDefinition::from_installed(&plugin))
    }

    fn plugin_enabled(&self, plugin_id: &PluginId) -> bool {
        PluginGateway::plugin_enabled(self, plugin_id)
    }

    fn data_directory(&self, plugin_id: &PluginId) -> Result<PathBuf, GatewayFailure> {
        PluginGateway::data_directory(self, plugin_id).map_err(GatewayFailure::from)
    }

    async fn ensure_running(
        &self,
        plugin_id: &PluginId,
        wait: Duration,
    ) -> Result<Self::Connection, GatewayFailure> {
        PluginGateway::ensure_running(self, plugin_id, wait)
            .await
            .map_err(GatewayFailure::from)
    }

    async fn stop_if_idle(&self, plugin_id: &PluginId) -> Result<(), GatewayFailure> {
        PluginGateway::stop_if_idle(self, plugin_id)
            .await
            .map_err(GatewayFailure::from)
    }
}
