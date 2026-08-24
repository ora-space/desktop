mod runtime;
mod scan;
mod state;
mod uninstall;

pub use runtime::{DenoPluginRuntime, DenoPluginRuntimeLauncher, PluginRuntimeTimeouts};
use state::{
    EnabledRuntime, LifecycleState, ManagedPluginState, discovered_plugin_contract,
    reconcile_persisted_state,
};
use uninstall::{plugin_data_root, stage_uninstall};

use ora_application::{Clock, PluginStateRepository, RepositoryError};
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, DisablePluginRequest, DisablePluginResponse,
    EnablePluginRequest, EnablePluginResponse, ListInstalledPluginsResponse, PluginDataDisposition,
    StopPluginRequest, StopPluginResponse, UninstallPluginRequest, UninstallPluginResponse,
};
use ora_domain::{PluginEnabledState, PluginId};
use ora_logging::ora_warn;
use ora_plugin_manager::{
    InstalledPlugin as DiscoveredPlugin, PluginConfigurationDeclarationValidity,
    PluginContribution, PluginManager,
};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// The Deno permissions granted to an agent plugin.
///
/// An agent plugin spawns and owns the agent CLI itself, so it needs `--allow-run` and everything
/// that CLI needs to work. That makes an agent plugin roughly as privileged as the host. This is a
/// deliberate, documented gap: capability narrowing for agent plugins is deferred until the agent
/// contract itself is proven, and closing it later changes only how the plugin is started.
const AGENT_PLUGIN_PERMISSIONS: [&str; 4] =
    ["--allow-run", "--allow-read", "--allow-env", "--allow-net"];

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
    /// Sandbox permissions the contribution kind requires, placed before the entrypoint.
    pub permissions: Vec<String>,
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
    /// The plugin-originated notification stream produced once per launched process.
    type Notifications: Send + 'static;

    /// Takes this launch's notification stream, yielding `None` once a consumer already owns it.
    ///
    /// A process emits one stream, so it is moved to its single consumer rather than cloned:
    /// splitting the frames of one plugin across two readers would silently lose traffic.
    fn take_notifications(&self) -> Option<Self::Notifications>;

    /// Stops the complete plugin process tree and resolves only after it has exited.
    fn stop(&self) -> impl Future<Output = Result<(), PluginRuntimeFailure>> + Send;

    /// Waits until the process exits and preserves whether shutdown was intentional.
    fn wait_for_exit(&self) -> impl Future<Output = PluginRuntimeExit> + Send + 'static;
}

/// Pairs one running plugin process with the notification stream of that same launch.
///
/// Handing both out together is what lets a consumer speak a protocol over the process without
/// owning its lifetime: the process stays lifecycle-owned while the stream is consumer-owned.
pub struct PluginAttachment<Runtime>
where
    Runtime: PluginRuntime,
{
    pub runtime: Runtime,
    pub notifications: Runtime::Notifications,
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
    #[error("plugin `{plugin_id}` has an invalid configuration declaration")]
    InvalidConfigurationDeclaration { plugin_id: String },
    #[error("plugin `{plugin_id}` did not reach a running runtime: {reason}")]
    RuntimeLaunch { plugin_id: String, reason: String },
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
        let manager = PluginManager::discover(&config.data_directory);
        // Discovery drops unusable packages silently, so startup is the only place an operator can
        // learn that an installed package never became a plugin.
        for issue in manager.discovery_issues() {
            ora_warn!(
                path = %issue.path().display(),
                issue_kind = issue.kind().as_str(),
                field_path = issue.field_path().unwrap_or(""),
                reason = issue.message(),
                "installed plugin manifest skipped during discovery"
            );
        }
        let installed = manager.installed_plugins().to_vec();
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

    /// Returns cached installed identity with configuration summaries resolved from current files.
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
                        &self.inner.config.data_directory,
                    )
                })
                .collect(),
        }
    }

    /// Returns the package root that owns one installed plugin's immutable declaration.
    pub fn installed_package_root(&self, plugin_id: &str) -> Result<PathBuf, PluginLifecycleError> {
        self.installed_plugin(plugin_id)
            .map(|plugin| plugin.package_root)
    }

    /// Persists eligibility and starts the runtime an enabled plugin is expected to have.
    ///
    /// Enabling is the user's statement that this plugin should be live, so it also owns the
    /// transition into a process: leaving an enabled plugin stopped would make the durable intent
    /// and the reported runtime disagree until something else happened to activate it.
    pub async fn enable_plugin(
        &self,
        request: EnablePluginRequest,
    ) -> Result<EnablePluginResponse, PluginLifecycleError> {
        let plugin_id = PluginId::new(&request.plugin_id);
        let operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.installed_plugin(&request.plugin_id)?;
        if matches!(
            &plugin.configuration_declaration,
            PluginConfigurationDeclarationValidity::Invalid { .. }
        ) {
            return Err(PluginLifecycleError::InvalidConfigurationDeclaration {
                plugin_id: request.plugin_id,
            });
        }
        self.inner
            .repository
            .set_plugin_enabled(
                &plugin_id,
                PluginEnabledState::Enabled,
                self.inner.clock.now_timestamp_millis(),
            )
            .map_err(PluginLifecycleError::Repository)?;

        let (response, launch) = {
            let mut state = self.write_state();
            let attempt = state.next_attempt;
            let managed = state
                .managed_by_id
                .entry(plugin_id.clone())
                .or_insert(ManagedPluginState::Disabled);
            // Only the transition out of disabled launches: re-enabling an already enabled plugin
            // must never restart a process an agent connection is currently speaking to.
            let launching = matches!(managed, ManagedPluginState::Disabled);
            if launching {
                *managed = ManagedPluginState::Enabled(EnabledRuntime::Starting { attempt });
            }
            let response = EnablePluginResponse {
                plugin: discovered_plugin_contract(
                    &plugin,
                    managed,
                    &self.inner.config.data_directory,
                ),
            };
            if launching {
                state.next_attempt = state.next_attempt.wrapping_add(1);
            }
            (response, launching.then_some(attempt))
        };
        if let Some(attempt) = launch {
            self.inner.publisher.publish_status_changed(&plugin_id);
            let inner = Arc::clone(&self.inner);
            let launched_id = plugin_id.clone();
            tokio::spawn(async move {
                complete_launch(inner, launched_id, plugin, attempt, operation).await;
            });
        }

        Ok(response)
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
                &self.inner.config.data_directory,
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
                        plugin: discovered_plugin_contract(
                            &plugin,
                            managed,
                            &self.inner.config.data_directory,
                        ),
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
                    plugin: discovered_plugin_contract(
                        &plugin,
                        managed,
                        &self.inner.config.data_directory,
                    ),
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

    /// Returns a running plugin together with the unclaimed notification stream of that launch.
    ///
    /// This is how a protocol consumer, such as an agent connection, reaches a plugin process
    /// without owning it: the runtime stays lifecycle-owned, so enabling, stopping, scanning, and
    /// uninstalling keep deciding the process lifetime while the consumer only reads its stream.
    /// A live process whose stream a previous consumer already took is restarted rather than
    /// shared, because one process emits exactly one stream.
    pub async fn attach_runtime(
        &self,
        plugin_id: &PluginId,
    ) -> Result<PluginAttachment<RuntimeLauncher::Runtime>, PluginLifecycleError> {
        let _operation = self.acquire_operation(plugin_id).await;
        let plugin = self.installed_plugin(plugin_id.as_ref())?;
        let reusable = {
            let state = self.read_state();
            match state.managed_by_id.get(plugin_id) {
                Some(ManagedPluginState::Disabled) | None => {
                    return Err(PluginLifecycleError::PluginDisabled {
                        plugin_id: plugin_id.to_string(),
                    });
                }
                Some(ManagedPluginState::Enabled(EnabledRuntime::Running { attempt, runtime })) => {
                    Some((*attempt, runtime.clone()))
                }
                Some(ManagedPluginState::Enabled(
                    EnabledRuntime::Stopped
                    | EnabledRuntime::Starting { .. }
                    | EnabledRuntime::Failed { .. },
                )) => None,
            }
        };
        if let Some((attempt, runtime)) = reusable {
            if let Some(notifications) = runtime.take_notifications() {
                return Ok(PluginAttachment {
                    runtime,
                    notifications,
                });
            }
            runtime
                .stop()
                .await
                .map_err(|source| PluginLifecycleError::RuntimeStop {
                    plugin_id: plugin_id.to_string(),
                    source,
                })?;
            transition_to_stopped(Arc::clone(&self.inner), plugin_id.clone(), attempt);
        }

        let attempt = {
            let mut state = self.write_state();
            let attempt = state.next_attempt;
            state.next_attempt = state.next_attempt.wrapping_add(1);
            state.managed_by_id.insert(
                plugin_id.clone(),
                ManagedPluginState::Enabled(EnabledRuntime::Starting { attempt }),
            );
            attempt
        };
        self.inner.publisher.publish_status_changed(plugin_id);
        launch_and_settle(Arc::clone(&self.inner), plugin_id.clone(), plugin, attempt).await;

        let runtime = {
            let state = self.read_state();
            match state.managed_by_id.get(plugin_id) {
                Some(ManagedPluginState::Enabled(EnabledRuntime::Running {
                    attempt: current,
                    runtime,
                })) if *current == attempt => runtime.clone(),
                Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { reason })) => {
                    return Err(PluginLifecycleError::RuntimeLaunch {
                        plugin_id: plugin_id.to_string(),
                        reason: reason.clone(),
                    });
                }
                Some(ManagedPluginState::Disabled)
                | Some(ManagedPluginState::Enabled(
                    EnabledRuntime::Stopped
                    | EnabledRuntime::Starting { .. }
                    | EnabledRuntime::Running { .. },
                ))
                | None => {
                    return Err(PluginLifecycleError::RuntimeLaunch {
                        plugin_id: plugin_id.to_string(),
                        reason: "the launch was superseded before it could be attached".to_string(),
                    });
                }
            }
        };
        let notifications =
            runtime
                .take_notifications()
                .ok_or_else(|| PluginLifecycleError::RuntimeLaunch {
                    plugin_id: plugin_id.to_string(),
                    reason: "the launched runtime published no notification stream".to_string(),
                })?;

        Ok(PluginAttachment {
            runtime,
            notifications,
        })
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
            plugin: discovered_plugin_contract(&plugin, managed, &self.inner.config.data_directory),
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

        let staged = match &plugin {
            Some(plugin) => Some(stage_uninstall(
                &self.inner.config.data_directory,
                plugin,
                request.data_disposition,
            )?),
            None => None,
        };
        if let Err(error) = self.inner.repository.delete_plugin_state(&plugin_id) {
            if let Some(staged) = staged {
                staged.rollback()?;
            }
            return Err(PluginLifecycleError::Repository(error));
        }
        {
            let mut state = self.write_state();
            state
                .installed
                .retain(|plugin| plugin.id != request.plugin_id);
            state.managed_by_id.remove(&plugin_id);
        }
        self.inner.publisher.publish_status_changed(&plugin_id);

        if let Some(staged) = staged
            && let Err(error) = staged.cleanup()
        {
            ora_warn!(
                plugin_id = %request.plugin_id,
                %error,
                "plugin uninstall committed but staging cleanup will need retry"
            );
        }
        if let Some(plugin) = &plugin {
            if let Some(namespace_root) = plugin.package_root.parent().and_then(Path::parent) {
                let _ = std::fs::remove_dir(namespace_root);
            }
            if matches!(request.data_disposition, PluginDataDisposition::Delete)
                && let Ok(data_root) =
                    plugin_data_root(&self.inner.config.data_directory, &plugin.id)
                && let Some(namespace_root) = data_root.parent()
            {
                let _ = std::fs::remove_dir(namespace_root);
            }
        }

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

/// Completes one launch attempt and releases the plugin's operation queue afterwards.
///
/// The guard is owned here rather than by the caller because the launch outlives the request that
/// started it: releasing it earlier would let a stop or disable interleave with a live launch.
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
    launch_and_settle(inner, plugin_id, plugin, attempt).await;
}

/// Runs one launch attempt without allowing stale work to overwrite a newer transition.
async fn launch_and_settle<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>(
    inner: Arc<PluginLifecycleInner<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher>>,
    plugin_id: PluginId,
    plugin: DiscoveredPlugin,
    attempt: u64,
) where
    Repository: PluginStateRepository + Send + Sync + 'static,
    LifecycleClock: Clock + Send + Sync + 'static,
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
{
    let PluginContribution::Agent(_) = &plugin.contributes;
    let launch = inner
        .launcher
        .launch(PluginLaunchRequest {
            plugin_id: plugin_id.clone(),
            deno_path: inner.config.deno_path.clone(),
            entrypoint: plugin.package_root.join(plugin.main.to_path_buf()),
            permissions: AGENT_PLUGIN_PERMISSIONS.map(str::to_string).to_vec(),
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
