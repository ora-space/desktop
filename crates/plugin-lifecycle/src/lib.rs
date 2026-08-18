mod runtime;
mod scan;
mod state;

pub use runtime::{DenoPluginRuntime, DenoPluginRuntimeLauncher, PluginRuntimeTimeouts};
use state::{
    EnabledRuntime, LifecycleState, ManagedPluginState, discovered_plugin_contract,
    reconcile_persisted_state,
};

use ora_application::{Clock, PluginStateRepository, RepositoryError};
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, DisablePluginRequest, DisablePluginResponse,
    EnablePluginRequest, EnablePluginResponse, ListInstalledPluginsResponse, StopPluginRequest,
    StopPluginResponse, UninstallPluginRequest, UninstallPluginResponse,
};
use ora_domain::{PluginEnabledState, PluginId};
use ora_plugin_manager::{InstalledPlugin as DiscoveredPlugin, PluginManager};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Configures the filesystem and executable inputs needed by plugin lifecycle orchestration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLifecycleConfig {
    pub data_directory: PathBuf,
    pub deno_path: PathBuf,
}

/// Describes one concrete process launch after package discovery has resolved its entrypoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLaunchRequest {
    pub plugin_id: PluginId,
    pub deno_path: PathBuf,
    pub entrypoint: PathBuf,
}

/// Preserves the reason a plugin process could not start or stopped unexpectedly.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{reason}")]
pub struct PluginRuntimeFailure {
    reason: String,
}

/// Distinguishes an intentional process exit from an unexpected runtime failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRuntimeExit {
    Stopped,
    Failed(PluginRuntimeFailure),
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

/// Owns one launched plugin process through explicit stop and asynchronous failure observation.
pub trait PluginRuntime: Clone + Send + Sync + 'static {
    /// Stops the complete plugin process tree and resolves only after it has exited.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send;

    /// Waits until the process exits and preserves whether shutdown was intentional.
    fn wait_for_exit(&self) -> impl Future<Output = PluginRuntimeExit> + Send + 'static;
}

/// Launches plugin runtimes while allowing tests to replace the external process boundary.
pub trait PluginRuntimeLauncher: Clone + Send + Sync + 'static {
    type Runtime: PluginRuntime;

    /// Starts one resolved plugin entrypoint and returns after runtime readiness is established.
    fn launch(
        &self,
        request: PluginLaunchRequest,
    ) -> impl Future<Output = Result<Self::Runtime, PluginRuntimeFailure>> + Send;
}

/// Publishes cache invalidations after observable plugin lifecycle transitions.
pub trait PluginStatusPublisher: Clone + Send + Sync + 'static {
    /// Announces that consumers should query the installed-plugin snapshot again.
    fn publish_status_changed(&self, plugin_id: &PluginId);
}

/// Reports a failure while constructing or operating plugin lifecycle state.
#[derive(Debug, Error)]
pub enum PluginLifecycleError {
    #[error("plugin state repository operation failed")]
    Repository(#[source] RepositoryError),
    #[error("installed plugin `{plugin_id}` was not found")]
    PluginNotFound { plugin_id: String },
    #[error("plugin `{plugin_id}` must be enabled before activation")]
    PluginDisabled { plugin_id: String },
    #[error("failed to stop plugin `{plugin_id}`")]
    RuntimeStop {
        plugin_id: String,
        #[source]
        source: PluginRuntimeFailure,
    },
    #[error("failed to remove plugin package at `{path}`")]
    PackageRemoval {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Joins discovered identity, durable eligibility, and process-scoped runtime behind one seam.
#[derive(Clone)]
pub struct PluginLifecycle<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>
where
    RuntimeLauncher: PluginRuntimeLauncher,
{
    inner: Arc<PluginLifecycleInner<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>>,
}

struct PluginLifecycleInner<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>
where
    RuntimeLauncher: PluginRuntimeLauncher,
{
    state: RwLock<LifecycleState<RuntimeLauncher::Runtime>>,
    scan_lock: AsyncMutex<()>,
    operation_locks: Mutex<BTreeMap<PluginId, Arc<AsyncMutex<()>>>>,
    repository: Repository,
    clock: LifecycleClock,
    launcher: RuntimeLauncher,
    publisher: StatusPublisher,
    config: PluginLifecycleConfig,
}

impl<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>
    PluginLifecycle<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>
where
    Repository: PluginStateRepository + Send + Sync + 'static,
    LifecycleClock: Clock + Send + Sync + 'static,
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
{
    /// Scans installed packages once, reconciles orphan rows, and composes runtime dependencies.
    pub fn open(
        config: PluginLifecycleConfig,
        repository: Repository,
        clock: LifecycleClock,
        launcher: RuntimeLauncher,
        publisher: StatusPublisher,
    ) -> Result<Self, PluginLifecycleError> {
        let installed = PluginManager::discover(&config.data_directory)
            .installed_plugins()
            .to_vec();
        let managed_by_id = reconcile_persisted_state(&repository, &installed)?;

        Ok(Self {
            inner: Arc::new(PluginLifecycleInner {
                state: RwLock::new(LifecycleState {
                    installed,
                    managed_by_id,
                    next_attempt: 1,
                }),
                scan_lock: AsyncMutex::new(()),
                operation_locks: Mutex::new(BTreeMap::new()),
                repository,
                clock,
                launcher,
                publisher,
                config,
            }),
        })
    }

    /// Returns the cached installed snapshot without touching the filesystem.
    pub fn list_installed_plugins(&self) -> ListInstalledPluginsResponse {
        let state = self.read_state();
        ListInstalledPluginsResponse {
            plugins: state
                .installed
                .iter()
                .map(|plugin| {
                    let plugin_id = PluginId::new(&plugin.id);
                    discovered_plugin_contract(
                        plugin,
                        state
                            .managed_by_id
                            .get(&plugin_id)
                            .unwrap_or(&ManagedPluginState::Disabled),
                    )
                })
                .collect(),
        }
    }

    /// Persists eligibility without changing an existing enabled runtime state.
    pub async fn enable_plugin(
        &self,
        request: EnablePluginRequest,
    ) -> Result<EnablePluginResponse, PluginLifecycleError> {
        let plugin_id = PluginId::new(&request.plugin_id);
        let _operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.installed_plugin(&request.plugin_id)?;
        self.inner
            .repository
            .set_plugin_enabled(
                &plugin_id,
                PluginEnabledState::Enabled,
                self.inner.clock.now_timestamp_millis(),
            )
            .map_err(PluginLifecycleError::Repository)?;

        let (plugin, changed) = {
            let mut state = self.write_state();
            let managed = state
                .managed_by_id
                .entry(plugin_id.clone())
                .or_insert(ManagedPluginState::Disabled);
            let changed = matches!(managed, ManagedPluginState::Disabled);
            if changed {
                *managed = ManagedPluginState::Enabled(EnabledRuntime::Stopped);
            }
            (discovered_plugin_contract(&plugin, managed), changed)
        };
        if changed {
            self.inner.publisher.publish_status_changed(&plugin_id);
        }

        Ok(EnablePluginResponse { plugin })
    }

    /// Persists ineligibility for an already-stopped plugin.
    pub async fn disable_plugin(
        &self,
        request: DisablePluginRequest,
    ) -> Result<DisablePluginResponse, PluginLifecycleError> {
        let plugin_id = PluginId::new(&request.plugin_id);
        let _operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.installed_plugin(&request.plugin_id)?;
        let running = {
            let state = self.read_state();
            match state.managed_by_id.get(&plugin_id) {
                Some(ManagedPluginState::Enabled(EnabledRuntime::Running { runtime, .. })) => {
                    Some(runtime.clone())
                }
                Some(ManagedPluginState::Enabled(EnabledRuntime::Starting { .. })) => {
                    // Launch normally owns this operation lock until Starting has resolved.
                    None
                }
                Some(ManagedPluginState::Disabled)
                | Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped))
                | Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { .. }))
                | None => None,
            }
        };
        if let Some(runtime) = running {
            runtime
                .stop()
                .await
                .map_err(|source| PluginLifecycleError::RuntimeStop {
                    plugin_id: request.plugin_id.clone(),
                    source,
                })?;
        }
        let persisted = self
            .inner
            .repository
            .find_plugin_state(&plugin_id)
            .map_err(PluginLifecycleError::Repository)?;
        // Missing durable state already means disabled, so only enable may create the first row.
        if matches!(
            persisted,
            Some(state) if state.enabled == PluginEnabledState::Enabled
        ) {
            self.inner
                .repository
                .set_plugin_enabled(
                    &plugin_id,
                    PluginEnabledState::Disabled,
                    self.inner.clock.now_timestamp_millis(),
                )
                .map_err(PluginLifecycleError::Repository)?;
        }

        let changed = {
            let mut state = self.write_state();
            let previous = state
                .managed_by_id
                .insert(plugin_id.clone(), ManagedPluginState::Disabled);
            !matches!(previous, Some(ManagedPluginState::Disabled))
        };
        if changed {
            self.inner.publisher.publish_status_changed(&plugin_id);
        }

        Ok(DisablePluginResponse {
            plugin: discovered_plugin_contract(
                &plugin,
                &ManagedPluginState::<RuntimeLauncher::Runtime>::Disabled,
            ),
        })
    }

    /// Starts an enabled plugin asynchronously and returns its immediate starting state.
    pub async fn activate_plugin(
        &self,
        request: ActivatePluginRequest,
    ) -> Result<ActivatePluginResponse, PluginLifecycleError> {
        let plugin_id = PluginId::new(&request.plugin_id);
        let operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.installed_plugin(&request.plugin_id)?;
        let (attempt, response) = {
            let mut state = self.write_state();
            let attempt = state.next_attempt;
            state.next_attempt = state.next_attempt.wrapping_add(1);
            let managed = state
                .managed_by_id
                .entry(plugin_id.clone())
                .or_insert(ManagedPluginState::Disabled);
            match managed {
                ManagedPluginState::Disabled => {
                    return Err(PluginLifecycleError::PluginDisabled {
                        plugin_id: request.plugin_id,
                    });
                }
                ManagedPluginState::Enabled(EnabledRuntime::Starting { .. })
                | ManagedPluginState::Enabled(EnabledRuntime::Running { .. }) => {
                    return Ok(ActivatePluginResponse {
                        plugin: discovered_plugin_contract(&plugin, managed),
                    });
                }
                ManagedPluginState::Enabled(EnabledRuntime::Stopped)
                | ManagedPluginState::Enabled(EnabledRuntime::Failed { .. }) => {
                    *managed = ManagedPluginState::Enabled(EnabledRuntime::Starting { attempt });
                }
            }
            (
                attempt,
                ActivatePluginResponse {
                    plugin: discovered_plugin_contract(&plugin, managed),
                },
            )
        };
        self.inner.publisher.publish_status_changed(&plugin_id);

        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            complete_launch(inner, plugin_id, plugin, attempt, operation).await;
        });

        Ok(response)
    }

    /// Stops one runtime to completion without changing durable eligibility.
    pub async fn stop_plugin(
        &self,
        request: StopPluginRequest,
    ) -> Result<StopPluginResponse, PluginLifecycleError> {
        let plugin_id = PluginId::new(&request.plugin_id);
        let _operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.installed_plugin(&request.plugin_id)?;
        let running = {
            let mut state = self.write_state();
            let managed = state
                .managed_by_id
                .entry(plugin_id.clone())
                .or_insert(ManagedPluginState::Disabled);
            match managed {
                ManagedPluginState::Disabled
                | ManagedPluginState::Enabled(EnabledRuntime::Stopped) => None,
                ManagedPluginState::Enabled(EnabledRuntime::Starting { .. }) => {
                    // Launch normally owns this operation lock until Starting has resolved.
                    *managed = ManagedPluginState::Enabled(EnabledRuntime::Stopped);
                    self.inner.publisher.publish_status_changed(&plugin_id);
                    None
                }
                ManagedPluginState::Enabled(EnabledRuntime::Failed { .. }) => {
                    *managed = ManagedPluginState::Enabled(EnabledRuntime::Stopped);
                    self.inner.publisher.publish_status_changed(&plugin_id);
                    None
                }
                ManagedPluginState::Enabled(EnabledRuntime::Running { attempt, runtime }) => {
                    Some((*attempt, runtime.clone()))
                }
            }
        };

        if let Some((attempt, runtime)) = running {
            runtime
                .stop()
                .await
                .map_err(|source| PluginLifecycleError::RuntimeStop {
                    plugin_id: request.plugin_id,
                    source,
                })?;
            let transitioned = {
                let mut state = self.write_state();
                if let Some(managed) = state.managed_by_id.get_mut(&plugin_id) {
                    let owns_attempt = matches!(
                        managed,
                        ManagedPluginState::Enabled(EnabledRuntime::Running {
                            attempt: current,
                            ..
                        }) if *current == attempt
                    );
                    if owns_attempt {
                        *managed = ManagedPluginState::Enabled(EnabledRuntime::Stopped);
                    }
                    owns_attempt
                } else {
                    false
                }
            };
            if transitioned {
                self.inner.publisher.publish_status_changed(&plugin_id);
            }
        }

        let state = self.read_state();
        let managed = state
            .managed_by_id
            .get(&plugin_id)
            .unwrap_or(&ManagedPluginState::Disabled);
        Ok(StopPluginResponse {
            plugin: discovered_plugin_contract(&plugin, managed),
        })
    }

    /// Stops a running plugin before physically removing its package and durable state.
    pub async fn uninstall_plugin(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, PluginLifecycleError> {
        let plugin_id = PluginId::new(&request.plugin_id);
        let _operation = self.acquire_operation(&plugin_id).await;
        let plugin = self
            .read_state()
            .installed
            .iter()
            .find(|plugin| plugin.id == request.plugin_id)
            .cloned();
        let persisted = self
            .inner
            .repository
            .find_plugin_state(&plugin_id)
            .map_err(PluginLifecycleError::Repository)?;
        if plugin.is_none() && persisted.is_none() {
            return Err(PluginLifecycleError::PluginNotFound {
                plugin_id: request.plugin_id,
            });
        }

        let running = {
            let state = self.read_state();
            match state.managed_by_id.get(&plugin_id) {
                Some(ManagedPluginState::Enabled(EnabledRuntime::Running { attempt, runtime })) => {
                    Some((*attempt, runtime.clone()))
                }
                Some(ManagedPluginState::Disabled)
                | Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped))
                | Some(ManagedPluginState::Enabled(EnabledRuntime::Starting { .. }))
                | Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { .. }))
                | None => None,
            }
        };
        if let Some((attempt, runtime)) = running {
            runtime
                .stop()
                .await
                .map_err(|source| PluginLifecycleError::RuntimeStop {
                    plugin_id: request.plugin_id.clone(),
                    source,
                })?;
            transition_to_stopped(Arc::clone(&self.inner), plugin_id.clone(), attempt);
        }

        self.inner
            .repository
            .delete_plugin_state(&plugin_id)
            .map_err(PluginLifecycleError::Repository)?;
        if let Some(plugin) = &plugin
            && plugin.package_root.exists()
        {
            std::fs::remove_dir_all(&plugin.package_root).map_err(|source| {
                PluginLifecycleError::PackageRemoval {
                    path: plugin.package_root.clone(),
                    source,
                }
            })?;
        }
        {
            let mut state = self.write_state();
            state
                .installed
                .retain(|plugin| plugin.id != request.plugin_id);
            state.managed_by_id.remove(&plugin_id);
        }
        self.inner.publisher.publish_status_changed(&plugin_id);

        Ok(UninstallPluginResponse {
            plugin_id: request.plugin_id,
        })
    }

    /// Loads one installed package from the cached discovery snapshot.
    fn installed_plugin(&self, plugin_id: &str) -> Result<DiscoveredPlugin, PluginLifecycleError> {
        self.read_state()
            .installed
            .iter()
            .find(|plugin| plugin.id == plugin_id)
            .cloned()
            .ok_or_else(|| PluginLifecycleError::PluginNotFound {
                plugin_id: plugin_id.to_string(),
            })
    }

    /// Acquires the independent queue associated with one plugin identifier.
    async fn acquire_operation(&self, plugin_id: &PluginId) -> OwnedMutexGuard<()> {
        let operation_lock = {
            let mut locks = self
                .inner
                .operation_locks
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            Arc::clone(
                locks
                    .entry(plugin_id.clone())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };

        operation_lock.lock_owned().await
    }

    /// Reads lifecycle state while recovering from a panicked thread's poisoned guard.
    fn read_state(&self) -> RwLockReadGuard<'_, LifecycleState<RuntimeLauncher::Runtime>> {
        self.inner
            .state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Mutates lifecycle state while recovering from a panicked thread's poisoned guard.
    fn write_state(&self) -> RwLockWriteGuard<'_, LifecycleState<RuntimeLauncher::Runtime>> {
        self.inner
            .state
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Completes one launch attempt without allowing stale work to overwrite a newer transition.
async fn complete_launch<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>(
    inner: Arc<PluginLifecycleInner<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>>,
    plugin_id: PluginId,
    plugin: DiscoveredPlugin,
    attempt: u64,
    _operation: OwnedMutexGuard<()>,
) where
    Repository: PluginStateRepository + Send + Sync + 'static,
    LifecycleClock: Clock + Send + Sync + 'static,
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
{
    let launch = inner
        .launcher
        .launch(PluginLaunchRequest {
            plugin_id: plugin_id.clone(),
            deno_path: inner.config.deno_path.clone(),
            entrypoint: plugin.package_root.join(plugin.main.to_path_buf()),
        })
        .await;

    match launch {
        Ok(runtime) => {
            let transitioned = {
                let mut state = inner.state.write().unwrap_or_else(PoisonError::into_inner);
                if let Some(managed) = state.managed_by_id.get_mut(&plugin_id) {
                    let owns_attempt = matches!(
                        managed,
                        ManagedPluginState::Enabled(EnabledRuntime::Starting {
                            attempt: current,
                        }) if *current == attempt
                    );
                    if owns_attempt {
                        *managed = ManagedPluginState::Enabled(EnabledRuntime::Running {
                            attempt,
                            runtime: runtime.clone(),
                        });
                    }
                    owns_attempt
                } else {
                    false
                }
            };
            if transitioned {
                inner.publisher.publish_status_changed(&plugin_id);
                let monitor_inner = Arc::clone(&inner);
                tokio::spawn(async move {
                    match runtime.wait_for_exit().await {
                        PluginRuntimeExit::Stopped => {
                            transition_to_stopped(monitor_inner, plugin_id, attempt);
                        }
                        PluginRuntimeExit::Failed(failure) => {
                            transition_to_failed(monitor_inner, plugin_id, attempt, failure);
                        }
                    }
                });
            } else {
                let _ = runtime.stop().await;
            }
        }
        Err(failure) => transition_to_failed(inner, plugin_id, attempt, failure),
    }
}

/// Records an intentional runtime exit only when its attempt still owns the running state.
fn transition_to_stopped<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>(
    inner: Arc<PluginLifecycleInner<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>>,
    plugin_id: PluginId,
    attempt: u64,
) where
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
{
    let transitioned = {
        let mut state = inner.state.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(managed) = state.managed_by_id.get_mut(&plugin_id) {
            let owns_attempt = matches!(
                managed,
                ManagedPluginState::Enabled(EnabledRuntime::Running {
                    attempt: current,
                    ..
                }) if *current == attempt
            );
            if owns_attempt {
                *managed = ManagedPluginState::Enabled(EnabledRuntime::Stopped);
            }
            owns_attempt
        } else {
            false
        }
    };
    if transitioned {
        inner.publisher.publish_status_changed(&plugin_id);
    }
}

/// Records a launch or runtime failure only when its attempt still owns the running state.
fn transition_to_failed<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>(
    inner: Arc<PluginLifecycleInner<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>>,
    plugin_id: PluginId,
    attempt: u64,
    failure: PluginRuntimeFailure,
) where
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
{
    let transitioned = {
        let mut state = inner.state.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(managed) = state.managed_by_id.get_mut(&plugin_id) {
            let owns_attempt = matches!(
                managed,
                ManagedPluginState::Enabled(EnabledRuntime::Starting {
                    attempt: current,
                }) | ManagedPluginState::Enabled(EnabledRuntime::Running {
                    attempt: current,
                    ..
                }) if *current == attempt
            );
            if owns_attempt {
                *managed = ManagedPluginState::Enabled(EnabledRuntime::Failed {
                    reason: failure.reason().to_string(),
                });
            }
            owns_attempt
        } else {
            false
        }
    };
    if transitioned {
        inner.publisher.publish_status_changed(&plugin_id);
    }
}

#[cfg(test)]
mod tests;
