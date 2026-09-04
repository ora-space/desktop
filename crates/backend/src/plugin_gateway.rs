//! The narrow plugin data-plane API the desktop surface layer consumes.
//!
//! Surfaces live in the desktop shell, but the plugin processes they talk to are owned by the
//! backend's lifecycle. This gateway exposes only the operations a surface needs: look up the
//! installed package, resolve the plugin's writable directory, obtain a connection to its process
//! (starting it on demand), stop it when idle, subscribe to the notifications plugin processes
//! emit (`ui/push`), and register the surface closer. The wider plugin management API stays on
//! `Backend`.

use crate::agent_runtime::AgentRuntimeManager;
use crate::plugin::PluginApi;
use ora_contracts::StopPluginRequest;
use ora_domain::PluginId;
use ora_plugin_lifecycle::{
    ConnectionError, DenoPluginRuntime, InboundNotification, PluginGenerationKey,
    PluginGenerationLease, PluginLifecycleError, SurfaceCloser, TraceContextGrant,
    TraceSessionGrant,
};
use ora_plugin_manager::InstalledPlugin;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::broadcast;

/// Reports why a gateway operation could not complete.
#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("plugin data directory could not be created")]
    DataDirectory(#[source] std::io::Error),
    #[error("plugin connection unavailable")]
    Connection(#[source] ConnectionError),
    #[error("plugin lifecycle operation failed")]
    Lifecycle(#[source] PluginLifecycleError),
    #[error("trace context could not be prepared: {0}")]
    TraceContext(String),
}

/// Plugin process access for the desktop surface layer.
pub struct PluginGateway {
    plugin: Arc<PluginApi>,
    agent_runtime: Arc<AgentRuntimeManager>,
}

impl PluginGateway {
    /// Wraps the backend's plugin composition.
    pub(crate) fn new(plugin: Arc<PluginApi>, agent_runtime: Arc<AgentRuntimeManager>) -> Self {
        Self {
            plugin,
            agent_runtime,
        }
    }

    /// Returns the installed package for `plugin_id` from the cached discovery snapshot.
    pub fn installed_plugin(&self, plugin_id: &PluginId) -> Option<InstalledPlugin> {
        self.plugin.lifecycle().installed_plugin(plugin_id)
    }

    /// Creates the plugin's data directory if needed and returns it.
    ///
    /// The surface layer calls this before any download lands, so the directory exists even when
    /// the plugin process has never been started.
    pub fn data_directory(&self, plugin_id: &PluginId) -> Result<PathBuf, GatewayError> {
        self.plugin
            .lifecycle()
            .plugin_data_directories()
            .ensure(plugin_id)
            .map_err(GatewayError::DataDirectory)
    }

    /// Starts the plugin if needed and waits up to `wait` for a running connection.
    pub async fn ensure_running(
        &self,
        plugin_id: &PluginId,
        wait: Duration,
    ) -> Result<PluginGenerationLease<DenoPluginRuntime>, GatewayError> {
        self.plugin
            .lifecycle()
            .ensure_running(plugin_id, wait)
            .await
            .map_err(GatewayError::Connection)
    }

    /// Returns a connection to the currently running generation without starting anything.
    pub fn connection(
        &self,
        plugin_id: &PluginId,
    ) -> Result<PluginGenerationLease<DenoPluginRuntime>, GatewayError> {
        self.plugin
            .lifecycle()
            .connection(plugin_id)
            .map_err(GatewayError::Connection)
    }

    /// Issues an opaque invocation context bound to the exact consumer process generation.
    pub fn issue_invocation_context(
        &self,
        plugin_id: PluginId,
        generation: PluginGenerationKey,
    ) -> String {
        self.plugin.issue_invocation_context(plugin_id, generation)
    }

    /// Grants a consumer context access to the selected Ora session's provider-declared traces.
    pub fn grant_session_trace_context(
        &self,
        context_id: &str,
        ora_session_id: &str,
    ) -> Result<(), GatewayError> {
        let current = self
            .agent_runtime
            .trace_session_binding(ora_session_id)
            .map_err(|error| GatewayError::TraceContext(error.to_string()))?;
        let mut bindings = self
            .agent_runtime
            .trace_session_catalog(ora_session_id)
            .map_err(|error| GatewayError::TraceContext(error.to_string()))?;
        if !bindings
            .iter()
            .any(|binding| binding.ora_session_id == current.ora_session_id)
        {
            bindings.push(current);
        }
        let sessions = bindings
            .into_iter()
            .filter_map(|binding| {
                let provider = self
                    .plugin
                    .lifecycle()
                    .connection(&binding.provider_plugin_id)
                    .ok()?;
                Some(TraceSessionGrant {
                    ora_session_id: binding.ora_session_id,
                    provider_plugin_id: binding.provider_plugin_id,
                    provider_generation: provider.key(),
                    provider_session_id: binding.provider_session_id,
                    workspace_root: binding.workspace_root,
                    providers: provider.registration().trace_providers,
                    label: binding.label,
                    updated_at_ms: binding.updated_at_ms,
                })
            })
            .collect::<Vec<_>>();
        if sessions.is_empty() {
            return Err(GatewayError::TraceContext(
                "no trace providers are currently available".to_string(),
            ));
        }
        let granted = self.plugin.grant_trace_context(
            context_id,
            TraceContextGrant {
                current_ora_session_id: ora_session_id.to_string(),
                sessions,
            },
        );
        if granted {
            Ok(())
        } else {
            Err(GatewayError::TraceContext(
                "invocation context is unavailable".to_string(),
            ))
        }
    }

    /// Revokes an invocation context after its owning page is closed.
    pub fn revoke_invocation_context(&self, context_id: &str) {
        self.plugin.revoke_invocation_context(context_id);
    }

    /// Stops the plugin process after its last surface instance has been idle long enough.
    ///
    /// The caller owns the instance registry and decides idleness; the lifecycle's per-plugin
    /// operation lock makes a concurrent reopen that races this stop safe, because that reopen
    /// goes through `ensure_running` and simply starts a new generation.
    pub async fn stop_if_idle(&self, plugin_id: &PluginId) -> Result<(), GatewayError> {
        self.plugin
            .lifecycle()
            .stop_plugin(StopPluginRequest {
                plugin_id: plugin_id.to_string(),
            })
            .await
            .map(|_| ())
            .map_err(GatewayError::Lifecycle)
    }

    /// Opens a receiver of every notification running plugin processes emit from now on.
    ///
    /// The surface host routes `ora/ui/push` from here to the panel webview that owns the session;
    /// a lagging receiver loses the oldest notifications rather than slowing the plugin.
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<InboundNotification> {
        self.plugin.subscribe_notifications()
    }

    /// Installs the desktop component that closes surfaces before a plugin stops or uninstalls.
    pub fn set_surface_closer(&self, closer: impl SurfaceCloser) {
        self.plugin.lifecycle().set_surface_closer(closer);
    }
}
