//! Hands callers a connection to one running plugin process, starting it on demand.
//!
//! A connection is pinned to one process generation. Callers keep it only for the duration of
//! one interaction and re-resolve afterwards, so a restarted plugin is never addressed through
//! a handle that belonged to its predecessor.

use crate::ports::{PluginCallError, PluginRuntime, PluginRuntimeLauncher, PluginStatusPublisher};
use crate::state::{EnabledRuntime, ManagedPluginState};
use crate::{PluginLifecycle, PluginLifecycleError, PluginNotificationSink};
use ora_application::{Clock, PluginStateRepository};
use ora_contracts::ActivatePluginRequest;
use ora_domain::PluginId;
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::watch;
use tokio::time::timeout;

/// Identifies one process generation of a plugin; equal to the lifecycle launch attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PluginGeneration(pub u64);

/// Reports why no connection to a plugin process could be produced.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConnectionError {
    #[error("plugin is not installed")]
    NotFound,
    #[error("plugin is disabled")]
    Disabled,
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
#[derive(Clone)]
pub struct PluginConnection<Runtime: PluginRuntime> {
    plugin_id: PluginId,
    generation: PluginGeneration,
    runtime: Runtime,
}

impl<Runtime: PluginRuntime> PluginConnection<Runtime> {
    /// Returns the plugin this connection addresses.
    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    /// Returns the process generation this connection is pinned to.
    pub fn generation(&self) -> PluginGeneration {
        self.generation
    }

    /// Invokes one registered method on this generation and returns its JSON result.
    pub async fn invoke(&self, method: &str, params: Value) -> Result<Value, PluginCallError> {
        self.runtime.invoke(method, params).await
    }

    /// Sends one notification to this generation.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), PluginCallError> {
        self.runtime.notify(method, params).await
    }
}

impl<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher, NotificationSink>
    PluginLifecycle<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher, NotificationSink>
where
    Repository: PluginStateRepository + Send + Sync + 'static,
    LifecycleClock: Clock + Send + Sync + 'static,
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
    NotificationSink: PluginNotificationSink,
{
    /// Returns a connection to the currently running generation without starting anything.
    pub fn connection(
        &self,
        plugin_id: &PluginId,
    ) -> Result<PluginConnection<RuntimeLauncher::Runtime>, ConnectionError> {
        let state = self.read_state();
        match state.managed(plugin_id) {
            None => Err(ConnectionError::NotFound),
            Some(ManagedPluginState::Disabled) => Err(ConnectionError::Disabled),
            Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped)) => {
                Err(ConnectionError::NotRunning)
            }
            Some(ManagedPluginState::Enabled(EnabledRuntime::Starting { .. })) => {
                Err(ConnectionError::NotReady)
            }
            Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { reason })) => {
                Err(ConnectionError::Failed(reason.clone()))
            }
            Some(ManagedPluginState::Enabled(EnabledRuntime::Running { attempt, runtime })) => {
                Ok(PluginConnection {
                    plugin_id: plugin_id.clone(),
                    generation: PluginGeneration(*attempt),
                    runtime: runtime.clone(),
                })
            }
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
    ) -> Result<PluginConnection<RuntimeLauncher::Runtime>, ConnectionError> {
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
    ) -> Result<PluginConnection<RuntimeLauncher::Runtime>, ConnectionError> {
        let mut activated = false;
        loop {
            let snapshot = status.borrow_and_update().clone();
            match snapshot {
                None => return Err(ConnectionError::NotFound),
                Some(ManagedPluginState::Disabled) => return Err(ConnectionError::Disabled),
                Some(ManagedPluginState::Enabled(EnabledRuntime::Running { attempt, runtime })) => {
                    return Ok(PluginConnection {
                        plugin_id: plugin_id.clone(),
                        generation: PluginGeneration(attempt),
                        runtime,
                    });
                }
                Some(ManagedPluginState::Enabled(EnabledRuntime::Starting { .. })) => {}
                Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped))
                | Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { .. }))
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
                        Err(PluginLifecycleError::PluginDisabled { .. }) => {
                            return Err(ConnectionError::Disabled);
                        }
                        Err(PluginLifecycleError::PluginNotFound { .. }) => {
                            return Err(ConnectionError::NotFound);
                        }
                        Err(
                            error @ (PluginLifecycleError::Repository(_)
                            | PluginLifecycleError::RuntimeStop { .. }
                            | PluginLifecycleError::PackageRemoval { .. }),
                        ) => return Err(ConnectionError::Failed(error.to_string())),
                    }
                }
                Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { reason })) => {
                    return Err(ConnectionError::Failed(reason));
                }
                // Stopped after our own activation means another operation stopped it first.
                Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped)) => {
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
