//! The plugin data-plane port the surface layer depends on.
//!
//! `ora-backend` exposes a concrete `PluginGateway`; this trait mirrors the subset the surface
//! host uses so the service can be driven by a fake in tests without a database or a Deno
//! process. Production binds it to `Arc<ora_backend::PluginGateway>`.

use ora_backend::{GatewayError, PluginGateway};
use ora_domain::PluginId;
use ora_plugin_lifecycle::{
    ConnectionError, InboundNotification, PluginCallError, PluginConnection, PluginGeneration,
    PluginRuntime,
};
use ora_plugin_manager::{InstalledSurface, PluginContribution};
use serde_json::Value;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::broadcast;

/// A live connection to one plugin process generation.
///
/// Implementations forward JSON-RPC calls to the process; the surface layer never needs more
/// than the generation (for logging and notification params) and the two call shapes.
pub trait SurfaceConnection: Clone + Send + Sync + 'static {
    /// Returns the process generation this connection talks to.
    fn generation(&self) -> PluginGeneration;

    /// Sends a request and waits for the plugin's result.
    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, PluginCallError>> + Send;

    /// Sends a notification without waiting for a result.
    fn notify(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<(), PluginCallError>> + Send;
}

impl<R: PluginRuntime> SurfaceConnection for PluginConnection<R> {
    fn generation(&self) -> PluginGeneration {
        PluginConnection::generation(self)
    }

    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, PluginCallError>> + Send {
        PluginConnection::invoke(self, method, params)
    }

    fn notify(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<(), PluginCallError>> + Send {
        PluginConnection::notify(self, method, params)
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

    /// Returns the surfaces an installed plugin declares (`None` when it is not installed; an
    /// agent plugin yields an empty list), enabled or not.
    fn installed_surfaces(&self, plugin_id: &PluginId) -> Option<Vec<InstalledSurface>>;

    /// Reports whether the plugin is installed and enabled.
    fn plugin_enabled(&self, plugin_id: &PluginId) -> bool;

    /// Creates and returns `<data-dir>/plugin-data/<plugin_id>`.
    fn data_directory(&self, plugin_id: &PluginId) -> Result<PathBuf, GatewayFailure>;

    /// Starts the plugin if needed and waits up to `wait` for a running connection.
    fn ensure_running(
        &self,
        plugin_id: &PluginId,
        wait: Duration,
    ) -> impl Future<Output = Result<Self::Connection, GatewayFailure>> + Send;

    /// Returns the running connection without starting anything.
    fn connection(&self, plugin_id: &PluginId) -> Result<Self::Connection, GatewayFailure>;

    /// Stops the plugin process; the caller has already verified that no instance is left.
    fn stop_if_idle(
        &self,
        plugin_id: &PluginId,
    ) -> impl Future<Output = Result<(), GatewayFailure>> + Send;

    /// Opens a receiver of every notification running plugin processes emit from now on.
    fn subscribe_notifications(&self) -> broadcast::Receiver<InboundNotification>;
}

impl SurfacePluginGateway for Arc<PluginGateway> {
    type Connection = PluginConnection<ora_plugin_lifecycle::DenoPluginRuntime>;

    fn installed_surfaces(&self, plugin_id: &PluginId) -> Option<Vec<InstalledSurface>> {
        PluginGateway::installed_plugin(self, plugin_id).map(|plugin| match plugin.contributes {
            PluginContribution::Ui(ui) => ui.surfaces,
            PluginContribution::Agent(_) => vec![],
        })
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

    fn connection(&self, plugin_id: &PluginId) -> Result<Self::Connection, GatewayFailure> {
        PluginGateway::connection(self, plugin_id).map_err(GatewayFailure::from)
    }

    async fn stop_if_idle(&self, plugin_id: &PluginId) -> Result<(), GatewayFailure> {
        PluginGateway::stop_if_idle(self, plugin_id)
            .await
            .map_err(GatewayFailure::from)
    }

    fn subscribe_notifications(&self) -> broadcast::Receiver<InboundNotification> {
        PluginGateway::subscribe_notifications(self)
    }
}
