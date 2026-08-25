mod connection;
mod data_dir;
mod launch;
mod permissions;
mod ports;
mod registration;
mod runtime;
mod scan;
mod state;
mod storage;
mod surface_closer;
mod uninstall;

pub use connection::{ConnectionError, PluginGenerationKey, PluginGenerationLease};
pub use data_dir::PluginDataDirectories;
pub use ora_plugin_runtime::{PluginNotification, PluginRegistration};
pub use permissions::{
    DenoPermission, PermissionFlagError, ReadScope, agent_permissions, permissions_for,
};
pub use ports::{
    InboundNotification, LaunchedRuntime, PluginCallError, PluginLaunchRequest,
    PluginNotificationSink, PluginRuntime, PluginRuntimeExit, PluginRuntimeFailure,
    PluginRuntimeLauncher, PluginStatusPublisher,
};
pub use registration::validate_registration;
pub use runtime::{DenoPluginRuntime, DenoPluginRuntimeLauncher, PluginRuntimeTimeouts};
pub use storage::{
    MAX_STORAGE_FILE_BYTES, PluginStorage, STORAGE_LIST_METHOD, STORAGE_READ_METHOD,
    STORAGE_REMOVE_METHOD, STORAGE_WRITE_METHOD, StorageEntry, StorageEntryKind, StorageError,
    StorageErrorKind,
};
pub use surface_closer::SurfaceCloser;

use launch::{complete_launch, transition_to_stopped};
use state::{
    EnabledRuntime, LifecycleState, ManagedPluginState, discovered_plugin_contract,
    reconcile_persisted_state,
};
use surface_closer::SurfaceCloserSlot;
use uninstall::{StagedUninstall, stage_uninstall};

use ora_application::{Clock, PluginStateRepository, RepositoryError};
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, DisablePluginRequest, DisablePluginResponse,
    EnablePluginRequest, EnablePluginResponse, ListInstalledPluginsResponse, PluginDataDisposition,
    StopPluginRequest, StopPluginResponse, UninstallPluginRequest, UninstallPluginResponse,
};
use ora_domain::{PluginEnabledState, PluginId};
use ora_logging::ora_warn;
use ora_plugin_config::ConfigurationService;
use ora_plugin_manager::{
    InstalledPlugin as DiscoveredPlugin, PluginConfigurationDeclarationValidity,
    PluginContribution, PluginManager,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Configures the filesystem and executable inputs needed by plugin lifecycle orchestration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLifecycleConfig {
    pub data_directory: PathBuf,
    pub deno_path: PathBuf,
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
    #[error("plugin `{plugin_id}` has no process to activate")]
    NoProcess { plugin_id: String },
    #[error("plugin `{plugin_id}` has an invalid configuration declaration")]
    InvalidConfigurationDeclaration { plugin_id: String },
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
    #[error("failed to stage plugin uninstall at `{path}`")]
    UninstallStaging {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Joins discovered identity, durable eligibility, and process-scoped runtime behind one seam.
#[derive(Clone)]
pub struct PluginLifecycle<
    Repository,
    LifecycleClock,
    RuntimeLauncher,
    StatusPublisher,
    NotificationSink,
> where
    RuntimeLauncher: PluginRuntimeLauncher,
{
    inner: Arc<
        PluginLifecycleInner<
            Repository,
            LifecycleClock,
            RuntimeLauncher,
            StatusPublisher,
            NotificationSink,
        >,
    >,
}

pub(crate) struct PluginLifecycleInner<
    Repository,
    LifecycleClock,
    RuntimeLauncher,
    StatusPublisher,
    NotificationSink,
> where
    RuntimeLauncher: PluginRuntimeLauncher,
{
    pub(crate) state: RwLock<LifecycleState<RuntimeLauncher::Runtime>>,
    scan_lock: AsyncMutex<()>,
    operation_locks: Mutex<BTreeMap<PluginId, Arc<AsyncMutex<()>>>>,
    pending_uninstall_cleanup: Mutex<Vec<StagedUninstall>>,
    repository: Repository,
    clock: LifecycleClock,
    pub(crate) launcher: RuntimeLauncher,
    pub(crate) publisher: StatusPublisher,
    pub(crate) sink: NotificationSink,
    pub(crate) data_directories: PluginDataDirectories,
    pub(crate) configuration: ConfigurationService,
    surface_closer: SurfaceCloserSlot,
    pub(crate) config: PluginLifecycleConfig,
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
    /// Scans installed packages once, reconciles orphan rows, and composes runtime dependencies.
    pub fn open(
        config: PluginLifecycleConfig,
        repository: Repository,
        clock: LifecycleClock,
        launcher: RuntimeLauncher,
        publisher: StatusPublisher,
        sink: NotificationSink,
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
                state: RwLock::new(LifecycleState::new(installed, managed_by_id)),
                scan_lock: AsyncMutex::new(()),
                operation_locks: Mutex::new(BTreeMap::new()),
                pending_uninstall_cleanup: Mutex::new(Vec::new()),
                repository,
                clock,
                launcher,
                publisher,
                sink,
                data_directories: PluginDataDirectories::new(&config.data_directory),
                configuration: ConfigurationService::new(config.data_directory.clone()),
                surface_closer: SurfaceCloserSlot::default(),
                config,
            }),
        })
    }

    /// Installs the host component that closes a plugin's surfaces before its process stops.
    ///
    /// Surfaces are owned by the desktop shell, which exists only after the backend (and this
    /// lifecycle) has been constructed, so the closer arrives late rather than at `open`.
    pub fn set_surface_closer(&self, closer: impl SurfaceCloser) {
        self.inner.surface_closer.install(closer);
    }

    /// Returns the per-plugin data directory manager shared with the surface layer.
    pub fn plugin_data_directories(&self) -> &PluginDataDirectories {
        &self.inner.data_directories
    }

    /// Returns the cached installed snapshot, reading current configuration summaries from disk.
    ///
    /// Package identity stays cached until an explicit scan; configuration completeness is not,
    /// because a later editor save would otherwise leave the list showing a stale Available state.
    pub fn list_installed_plugins(&self) -> ListInstalledPluginsResponse {
        let state = self.read_state();
        ListInstalledPluginsResponse {
            plugins: state
                .installed
                .iter()
                .map(|plugin| {
                    discovered_plugin_contract(
                        plugin,
                        state
                            .managed(&plugin.id)
                            .unwrap_or(&ManagedPluginState::Disabled),
                        &self.inner.configuration,
                    )
                })
                .collect(),
        }
    }

    /// Returns the package root that owns one installed plugin's immutable declaration.
    pub fn installed_package_root(&self, plugin_id: &str) -> Result<PathBuf, PluginLifecycleError> {
        self.require_installed(&parse_request_id(plugin_id)?)
            .map(|plugin| plugin.package_root)
    }

    /// Returns one installed package from the cached discovery snapshot, if present.
    pub fn installed_plugin(&self, plugin_id: &PluginId) -> Option<DiscoveredPlugin> {
        self.read_state()
            .installed
            .iter()
            .find(|plugin| plugin.id == *plugin_id)
            .cloned()
    }

    /// Persists eligibility and, for an agent plugin, starts the runtime it is expected to have.
    ///
    /// Enabling an agent plugin is the user's statement that its agent should be reachable, and
    /// the agent supervisor attaches to a running process rather than starting one, so enabling
    /// owns that transition: leaving it stopped would make the durable intent and the reported
    /// runtime disagree. A ui plugin's process is started on demand by its first surface and
    /// reaped when idle, so enabling one only records eligibility.
    pub async fn enable_plugin(
        &self,
        request: EnablePluginRequest,
    ) -> Result<EnablePluginResponse, PluginLifecycleError> {
        let plugin_id = parse_request_id(&request.plugin_id)?;
        let operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.require_installed(&plugin_id)?;
        if !has_valid_configuration_declaration(&plugin) {
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

        let (response, changed, launch) = {
            let mut state = self.write_state();
            // Only the transition out of disabled launches: re-enabling an already enabled plugin
            // must never restart a process a consumer is currently speaking to.
            let changed = matches!(
                state.managed(&plugin_id),
                Some(ManagedPluginState::Disabled) | None
            );
            let attempt = match (&plugin.contributes, changed) {
                (PluginContribution::Agent(_), true) => {
                    let attempt = state.next_attempt;
                    state.next_attempt = state.next_attempt.wrapping_add(1);
                    state.set_managed(
                        &plugin_id,
                        ManagedPluginState::Enabled(EnabledRuntime::Starting { attempt }),
                    );
                    Some(attempt)
                }
                (
                    PluginContribution::Workbench(_)
                    | PluginContribution::Webview(_)
                    | PluginContribution::Skill(_),
                    true,
                ) => {
                    state.set_managed(
                        &plugin_id,
                        ManagedPluginState::Enabled(EnabledRuntime::Stopped),
                    );
                    None
                }
                (
                    PluginContribution::Agent(_)
                    | PluginContribution::Workbench(_)
                    | PluginContribution::Webview(_)
                    | PluginContribution::Skill(_),
                    false,
                ) => None,
            };
            let managed = state
                .managed(&plugin_id)
                .unwrap_or(&ManagedPluginState::Disabled);
            (
                EnablePluginResponse {
                    plugin: discovered_plugin_contract(&plugin, managed, &self.inner.configuration),
                },
                changed,
                attempt,
            )
        };
        if changed {
            self.inner.publisher.publish_status_changed(&plugin_id);
        }
        if let Some(attempt) = launch {
            let inner = Arc::clone(&self.inner);
            tokio::spawn(async move {
                complete_launch(inner, plugin_id, plugin, attempt, operation).await;
            });
        }

        Ok(response)
    }

    /// Closes surfaces, stops the runtime if needed, then persists ineligibility.
    pub async fn disable_plugin(
        &self,
        request: DisablePluginRequest,
    ) -> Result<DisablePluginResponse, PluginLifecycleError> {
        let plugin_id = parse_request_id(&request.plugin_id)?;
        let _operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.require_installed(&plugin_id)?;
        self.inner.surface_closer.close_all(&plugin_id).await;
        let running = self.running_runtime(&plugin_id);
        if let Some((_, runtime)) = running {
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
            let changed = !matches!(
                state.managed(&plugin_id),
                Some(ManagedPluginState::Disabled)
            );
            state.set_managed(&plugin_id, ManagedPluginState::Disabled);
            changed
        };
        if changed {
            self.inner.publisher.publish_status_changed(&plugin_id);
        }

        Ok(DisablePluginResponse {
            plugin: discovered_plugin_contract(
                &plugin,
                &ManagedPluginState::<RuntimeLauncher::Runtime>::Disabled,
                &self.inner.configuration,
            ),
        })
    }

    /// Starts an enabled plugin asynchronously and returns its immediate starting state.
    pub async fn activate_plugin(
        &self,
        request: ActivatePluginRequest,
    ) -> Result<ActivatePluginResponse, PluginLifecycleError> {
        let plugin_id = parse_request_id(&request.plugin_id)?;
        let operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.require_installed(&plugin_id)?;
        if !has_valid_configuration_declaration(&plugin) {
            return Err(PluginLifecycleError::InvalidConfigurationDeclaration {
                plugin_id: request.plugin_id.clone(),
            });
        }
        // A webview plugin is configuration only; activating it would launch nothing and then
        // report a failure the user cannot act on.
        if plugin.contributes.entrypoint().is_none() {
            return Err(PluginLifecycleError::NoProcess {
                plugin_id: request.plugin_id,
            });
        }
        let (attempt, response) = {
            let mut state = self.write_state();
            let attempt = state.next_attempt;
            state.next_attempt = state.next_attempt.wrapping_add(1);
            match state.managed(&plugin_id) {
                Some(ManagedPluginState::Disabled) | None => {
                    return Err(PluginLifecycleError::PluginDisabled {
                        plugin_id: request.plugin_id,
                    });
                }
                Some(
                    managed @ (ManagedPluginState::Enabled(EnabledRuntime::Starting { .. })
                    | ManagedPluginState::Enabled(EnabledRuntime::Running { .. })),
                ) => {
                    return Ok(ActivatePluginResponse {
                        plugin: discovered_plugin_contract(
                            &plugin,
                            managed,
                            &self.inner.configuration,
                        ),
                    });
                }
                Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped))
                | Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { .. })) => {
                    let starting =
                        ManagedPluginState::Enabled(EnabledRuntime::Starting { attempt });
                    let response = ActivatePluginResponse {
                        plugin: discovered_plugin_contract(
                            &plugin,
                            &starting,
                            &self.inner.configuration,
                        ),
                    };
                    state.set_managed(&plugin_id, starting);
                    (attempt, response)
                }
            }
        };
        self.inner.publisher.publish_status_changed(&plugin_id);

        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            complete_launch(inner, plugin_id, plugin, attempt, operation).await;
        });

        Ok(response)
    }

    /// Closes surfaces and stops one runtime to completion without changing durable eligibility.
    pub async fn stop_plugin(
        &self,
        request: StopPluginRequest,
    ) -> Result<StopPluginResponse, PluginLifecycleError> {
        let plugin_id = parse_request_id(&request.plugin_id)?;
        let _operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.require_installed(&plugin_id)?;
        self.inner.surface_closer.close_all(&plugin_id).await;
        let running = {
            let mut state = self.write_state();
            match state.managed(&plugin_id) {
                Some(ManagedPluginState::Disabled)
                | Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped))
                | None => None,
                // Launch normally owns this operation lock until Starting has resolved, so a
                // Starting or Failed plugin can be marked stopped without touching a process.
                Some(ManagedPluginState::Enabled(EnabledRuntime::Starting { .. }))
                | Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { .. })) => {
                    state.set_managed(
                        &plugin_id,
                        ManagedPluginState::Enabled(EnabledRuntime::Stopped),
                    );
                    self.inner.publisher.publish_status_changed(&plugin_id);
                    None
                }
                Some(ManagedPluginState::Enabled(EnabledRuntime::Running { attempt, runtime })) => {
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
            transition_to_stopped(Arc::clone(&self.inner), plugin_id.clone(), attempt);
        }

        let state = self.read_state();
        let managed = state
            .managed(&plugin_id)
            .unwrap_or(&ManagedPluginState::Disabled);
        Ok(StopPluginResponse {
            plugin: discovered_plugin_contract(&plugin, managed, &self.inner.configuration),
        })
    }

    /// Closes surfaces and stops the runtime before removing the package, data, and durable state.
    pub async fn uninstall_plugin(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, PluginLifecycleError> {
        let plugin_id = parse_request_id(&request.plugin_id)?;
        let _operation = self.acquire_operation(&plugin_id).await;
        let plugin = self.installed_plugin(&plugin_id);
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

        // Surfaces close before the process stops and before the package disappears, all under
        // the same operation lock, so "uninstall while open" needs no extra coordination.
        self.inner.surface_closer.close_all(&plugin_id).await;
        if let Some((attempt, runtime)) = self.running_runtime(&plugin_id) {
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
        if plugin.is_none() && matches!(request.data_disposition, PluginDataDisposition::Delete) {
            self.inner
                .data_directories
                .remove(&plugin_id)
                .map_err(|source| PluginLifecycleError::PackageRemoval {
                    path: self.inner.data_directories.path_for(&plugin_id),
                    source,
                })?;
        }
        if let Err(error) = self.inner.repository.delete_plugin_state(&plugin_id) {
            if let Some(staged) = staged
                && let Err(rollback_error) = staged.rollback()
            {
                ora_warn!(
                    plugin_id = %request.plugin_id,
                    %error,
                    %rollback_error,
                    "plugin state deletion failed and staged uninstall rollback also failed"
                );
                return Err(rollback_error);
            }
            return Err(PluginLifecycleError::Repository(error));
        }
        {
            let mut state = self.write_state();
            state.installed.retain(|plugin| plugin.id != plugin_id);
            state.remove_managed(&plugin_id);
        }
        self.inner.publisher.publish_status_changed(&plugin_id);

        if let Some(staged) = staged
            && let Err(error) = staged.cleanup()
        {
            ora_warn!(
                plugin_id = %request.plugin_id,
                %error,
                "plugin uninstall committed but staging cleanup will be retried"
            );
            self.inner
                .pending_uninstall_cleanup
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(staged);
        }
        if let Some(plugin) = &plugin
            && let Some(namespace_root) = plugin.package_root.parent().and_then(Path::parent)
        {
            remove_empty_namespace_directory(namespace_root);
        }

        Ok(UninstallPluginResponse {
            plugin_id: request.plugin_id,
        })
    }

    /// Loads one installed package from the cached discovery snapshot or fails with not-found.
    fn require_installed(
        &self,
        plugin_id: &PluginId,
    ) -> Result<DiscoveredPlugin, PluginLifecycleError> {
        self.installed_plugin(plugin_id)
            .ok_or_else(|| PluginLifecycleError::PluginNotFound {
                plugin_id: plugin_id.to_string(),
            })
    }

    /// Returns the running attempt and runtime handle of one plugin, if it is running.
    fn running_runtime(&self, plugin_id: &PluginId) -> Option<(u64, RuntimeLauncher::Runtime)> {
        let state = self.read_state();
        match state.managed(plugin_id) {
            Some(ManagedPluginState::Enabled(EnabledRuntime::Running { attempt, runtime })) => {
                Some((*attempt, runtime.clone()))
            }
            // Launch normally owns the operation lock until Starting has resolved.
            Some(ManagedPluginState::Disabled)
            | Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped))
            | Some(ManagedPluginState::Enabled(EnabledRuntime::Starting { .. }))
            | Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { .. }))
            | None => None,
        }
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

/// Removes an empty namespace directory while retaining unexpected filesystem failures in logs.
fn remove_empty_namespace_directory(path: &Path) {
    if let Err(error) = std::fs::remove_dir(path)
        && !matches!(
            error.kind(),
            std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
        )
    {
        ora_warn!(path = %path.display(), %error, "could not remove empty plugin namespace directory");
    }
}

/// Keeps every launch path aligned with installation declaration validity.
pub(crate) fn has_valid_configuration_declaration(plugin: &DiscoveredPlugin) -> bool {
    !matches!(
        plugin.configuration_declaration,
        PluginConfigurationDeclarationValidity::Invalid { .. }
    )
}

/// Parses the canonical plugin id carried by a request.
///
/// A malformed id is reported as not found rather than as a distinct error: from the caller's
/// point of view no installed plugin answers to that spelling, and the error contract stays the
/// one the frontend already handles.
fn parse_request_id(plugin_id: &str) -> Result<PluginId, PluginLifecycleError> {
    PluginId::parse(plugin_id).map_err(|_| PluginLifecycleError::PluginNotFound {
        plugin_id: plugin_id.to_string(),
    })
}

#[cfg(test)]
mod data_plane_tests;
#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod tests;
