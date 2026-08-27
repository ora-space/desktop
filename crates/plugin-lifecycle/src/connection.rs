//! Hands callers a connection to one running plugin process, starting it on demand.
//!
//! A connection is pinned to one process generation. Callers keep it only for the duration of
//! one interaction and re-resolve afterwards, so a restarted plugin is never addressed through
//! a handle that belonged to its predecessor.

use crate::ports::{PluginCallError, PluginRuntime, PluginRuntimeLauncher, PluginStatusPublisher};
use crate::state::ManagedPluginState;
use crate::{PluginLifecycle, PluginLifecycleError, PluginNotificationSink};
use ora_contracts::ActivatePluginRequest;
use ora_domain::PluginId;
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::watch;
use tokio::time::timeout;

/// Identifies one process generation of a plugin; equal to the lifecycle launch attempt.
///
/// The key is a stable, non-callable identity: it is safe to keep in every surface state for
/// indexing and cleanup, unlike the lease, which only a ready surface may hold.
///
/// The key is a stable, non-callable identity: it is safe to keep in every surface state for
/// indexing and cleanup, unlike the lease, which only a ready surface may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginGenerationKey(pub u64);

/// Reports why no connection to a plugin process could be produced.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConnectionError {
    #[error("plugin is not installed")]
    NotFound,
    #[error("plugin has no process")]
    NoProcess,
    #[error("plugin failed: {0}")]
    Failed(String),
    #[error("plugin did not become ready in time")]
    Timeout,
    #[error("plugin is still starting")]
    NotReady,
    #[error("plugin is not running")]
    NotRunning,
}

/// A call handle bound to exactly one running process generation of a plugin.
///
/// The lease is the only way a consumer speaks to a process: it exposes the generation's
/// identity, the registration that generation published, and `invoke`, and never the raw
/// runtime's lifetime controls. It does not own restart policy and cannot revive a generation
/// that ended; once the process is gone, every call fails with `PluginUnavailable`.
#[derive(Clone)]
pub struct PluginGenerationLease<Runtime: PluginRuntime> {
    plugin_id: PluginId,
    generation: PluginGenerationKey,
    runtime: Runtime,
}

impl<Runtime: PluginRuntime> PluginGenerationLease<Runtime> {
    /// Returns the plugin this connection addresses.
    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    /// Returns the process generation this lease is pinned to.
    pub fn key(&self) -> PluginGenerationKey {
        self.generation
    }

    /// Returns the runtime handle of this generation for consumers that need more than calls.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Invokes one registered method on this generation and returns its JSON result.
    pub async fn invoke(&self, method: &str, params: Value) -> Result<Value, PluginCallError> {
        self.runtime.invoke(method, params).await
    }

    /// Returns the methods this generation registered at its handshake.
    pub fn registered_methods(&self) -> HashSet<String> {
        self.runtime.registration().methods
    }
}

impl<RuntimeLauncher, StatusPublisher, NotificationSink>
    PluginLifecycle<RuntimeLauncher, StatusPublisher, NotificationSink>
where
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
    NotificationSink: PluginNotificationSink,
{
    /// Returns a connection to the currently running generation without starting anything.
    pub fn connection(
        &self,
        plugin_id: &PluginId,
    ) -> Result<PluginGenerationLease<RuntimeLauncher::Runtime>, ConnectionError> {
        let state = self.read_state();
        match state.managed(plugin_id) {
            None => Err(ConnectionError::NotFound),
            Some(ManagedPluginState::Stopped) => Err(ConnectionError::NotRunning),
            Some(ManagedPluginState::Starting { .. }) => Err(ConnectionError::NotReady),
            Some(ManagedPluginState::Failed { reason }) => {
                Err(ConnectionError::Failed(reason.clone()))
            }
            Some(ManagedPluginState::Running { attempt, runtime }) => Ok(PluginGenerationLease {
                plugin_id: plugin_id.clone(),
                generation: PluginGenerationKey(*attempt),
                runtime: runtime.clone(),
            }),
        }
    }

    /// Activates the plugin when it is stopped or failed and waits until it is running.
    ///
    /// Surface opening and download dispatch both go through here, so a plugin process is only
    /// ever started by demand or by an explicit user action, never by a background poll.
    pub async fn ensure_running(
        &self,
        plugin_id: &PluginId,
        wait: Duration,
    ) -> Result<PluginGenerationLease<RuntimeLauncher::Runtime>, ConnectionError> {
        let mut status = self.write_state().subscribe(plugin_id);
        match timeout(wait, self.await_running(plugin_id, &mut status)).await {
            Ok(result) => result,
            Err(_) => Err(ConnectionError::Timeout),
        }
    }

    /// Follows one plugin's transitions, activating at most once, until a terminal answer.
    async fn await_running(
        &self,
        plugin_id: &PluginId,
        status: &mut watch::Receiver<Option<ManagedPluginState<RuntimeLauncher::Runtime>>>,
    ) -> Result<PluginGenerationLease<RuntimeLauncher::Runtime>, ConnectionError> {
        let mut activated = false;
        loop {
            let snapshot = status.borrow_and_update().clone();
            match snapshot {
                None => return Err(ConnectionError::NotFound),
                Some(ManagedPluginState::Running { attempt, runtime }) => {
                    return Ok(PluginGenerationLease {
                        plugin_id: plugin_id.clone(),
                        generation: PluginGenerationKey(attempt),
                        runtime,
                    });
                }
                Some(ManagedPluginState::Starting { .. }) => {}
                Some(ManagedPluginState::Stopped) | Some(ManagedPluginState::Failed { .. })
                    if !activated =>
                {
                    activated = true;
                    match self
                        .activate_plugin(ActivatePluginRequest {
                            plugin_id: plugin_id.to_string(),
                        })
                        .await
                    {
                        // The activation already moved the watch to Starting (or found it
                        // running); re-read instead of waiting for a further change.
                        Ok(_) => continue,
                        Err(PluginLifecycleError::PluginNotFound { .. }) => {
                            return Err(ConnectionError::NotFound);
                        }
                        Err(PluginLifecycleError::NoProcess { .. }) => {
                            return Err(ConnectionError::NoProcess);
                        }
                        Err(PluginLifecycleError::InvalidConfigurationDeclaration { .. }) => {
                            return Err(ConnectionError::Failed(
                                "plugin configuration declaration is invalid".to_string(),
                            ));
                        }
                        Err(
                            error @ (PluginLifecycleError::RuntimeStop { .. }
                            | PluginLifecycleError::PackageRemoval { .. }
                            | PluginLifecycleError::UninstallStaging { .. }),
                        ) => return Err(ConnectionError::Failed(error.to_string())),
                    }
                }
                Some(ManagedPluginState::Failed { reason }) => {
                    return Err(ConnectionError::Failed(reason));
                }
                // Stopped after our own activation means another operation stopped it first.
                Some(ManagedPluginState::Stopped) => {
                    return Err(ConnectionError::Failed(
                        "plugin stopped before it became ready".to_string(),
                    ));
                }
            }
            if status.changed().await.is_err() {
                return Err(ConnectionError::NotFound);
            }
        }
    }
}
