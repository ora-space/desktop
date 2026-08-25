use super::{
    EnabledRuntime, ManagedPluginState, PluginLifecycle, PluginLifecycleError,
    PluginNotificationSink, PluginRuntime, PluginRuntimeLauncher, PluginStatusPublisher,
    has_valid_configuration_declaration,
};
use ora_application::{Clock, PluginStateRepository};
use ora_contracts::{ScanPluginsRequest, ScanPluginsResponse};
use ora_domain::PluginEnabledState;
use ora_logging::ora_warn;
use ora_plugin_manager::PluginManager;
use std::collections::{BTreeMap, BTreeSet};

impl<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher, NotificationSink>
    PluginLifecycle<Repository, LifecycleClock, RuntimeLauncher, StatusPublisher, NotificationSink>
where
    Repository: PluginStateRepository + Send + Sync + 'static,
    LifecycleClock: Clock + Send + Sync + 'static,
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
    NotificationSink: PluginNotificationSink,
{
    /// Rebuilds the installed snapshot and rejoins durable eligibility on explicit request.
    pub async fn scan_plugins(
        &self,
        _request: ScanPluginsRequest,
    ) -> Result<ScanPluginsResponse, PluginLifecycleError> {
        let _scan = self.inner.scan_lock.lock().await;
        self.inner
            .pending_uninstall_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|staged| match staged.cleanup() {
                Ok(()) => false,
                Err(error) => {
                    ora_warn!(%error, "plugin uninstall staging cleanup retry failed");
                    true
                }
            });
        let cached_ids = self
            .read_state()
            .installed
            .iter()
            .map(|plugin| plugin.id.clone())
            .collect::<BTreeSet<_>>();
        let mut _operations = Vec::with_capacity(cached_ids.len());
        // Scan is a reconciliation barrier: waiting here favors one coherent snapshot over a
        // partial result while a cached plugin is still completing another lifecycle operation.
        for plugin_id in &cached_ids {
            _operations.push(self.acquire_operation(plugin_id).await);
        }

        let installed = PluginManager::discover(&self.inner.config.data_directory)
            .installed_plugins()
            .to_vec();
        let installed_ids = installed
            .iter()
            .map(|plugin| plugin.id.clone())
            .collect::<BTreeSet<_>>();
        let installed_by_id = installed
            .iter()
            .map(|plugin| (plugin.id.clone(), plugin))
            .collect::<BTreeMap<_, _>>();
        let removed_ids = cached_ids
            .difference(&installed_ids)
            .cloned()
            .collect::<Vec<_>>();
        let removed_runtimes = {
            let state = self.read_state();
            removed_ids
                .iter()
                .filter_map(|plugin_id| match state.managed(plugin_id) {
                    Some(ManagedPluginState::Enabled(EnabledRuntime::Running {
                        runtime, ..
                    })) => Some((plugin_id.clone(), runtime.clone())),
                    Some(ManagedPluginState::Disabled)
                    | Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped))
                    | Some(ManagedPluginState::Enabled(EnabledRuntime::Starting { .. }))
                    | Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { .. }))
                    | None => None,
                })
                .collect::<Vec<_>>()
        };
        for (plugin_id, runtime) in removed_runtimes {
            runtime
                .stop()
                .await
                .map_err(|source| PluginLifecycleError::RuntimeStop {
                    plugin_id: plugin_id.to_string(),
                    source,
                })?;
        }

        let mut persisted_by_id = BTreeMap::new();
        let mut orphaned_persisted_ids = Vec::new();
        for persisted in self
            .inner
            .repository
            .list_plugin_states()
            .map_err(PluginLifecycleError::Repository)?
        {
            if installed_ids.contains(&persisted.plugin_id) {
                let valid = installed_by_id
                    .get(&persisted.plugin_id)
                    .is_some_and(|plugin| has_valid_configuration_declaration(plugin));
                let enabled = if valid {
                    persisted.enabled
                } else {
                    if persisted.enabled == PluginEnabledState::Enabled {
                        self.inner
                            .repository
                            .set_plugin_enabled(
                                &persisted.plugin_id,
                                PluginEnabledState::Disabled,
                                self.inner.clock.now_timestamp_millis(),
                            )
                            .map_err(PluginLifecycleError::Repository)?;
                    }
                    PluginEnabledState::Disabled
                };
                persisted_by_id.insert(persisted.plugin_id, enabled);
            } else {
                orphaned_persisted_ids.push(persisted.plugin_id);
            }
        }

        let ineligible_runtimes = {
            let state = self.read_state();
            installed_ids
                .iter()
                .filter_map(|plugin_id| {
                    let persisted = persisted_by_id
                        .get(plugin_id)
                        .copied()
                        .unwrap_or(PluginEnabledState::Disabled);
                    if persisted == PluginEnabledState::Enabled {
                        return None;
                    }

                    match state.managed(plugin_id) {
                        Some(ManagedPluginState::Enabled(EnabledRuntime::Running {
                            runtime,
                            ..
                        })) => Some((plugin_id.clone(), runtime.clone())),
                        Some(ManagedPluginState::Disabled)
                        | Some(ManagedPluginState::Enabled(EnabledRuntime::Stopped))
                        | Some(ManagedPluginState::Enabled(EnabledRuntime::Starting { .. }))
                        | Some(ManagedPluginState::Enabled(EnabledRuntime::Failed { .. }))
                        | None => None,
                    }
                })
                .collect::<Vec<_>>()
        };
        for (plugin_id, runtime) in ineligible_runtimes {
            runtime
                .stop()
                .await
                .map_err(|source| PluginLifecycleError::RuntimeStop {
                    plugin_id: plugin_id.to_string(),
                    source,
                })?;
        }
        for plugin_id in orphaned_persisted_ids {
            self.inner
                .repository
                .delete_plugin_state(&plugin_id)
                .map_err(PluginLifecycleError::Repository)?;
        }

        let changed_ids = {
            let mut state = self.write_state();
            let previous_ids = state
                .installed
                .iter()
                .map(|plugin| plugin.id.clone())
                .collect::<BTreeSet<_>>();
            let mut managed_by_id = BTreeMap::new();
            let mut changed_ids = previous_ids
                .symmetric_difference(&installed_ids)
                .cloned()
                .collect::<BTreeSet<_>>();
            for plugin_id in &installed_ids {
                let previous = state.managed(plugin_id).cloned();
                let persisted = persisted_by_id
                    .get(plugin_id)
                    .copied()
                    .unwrap_or(PluginEnabledState::Disabled);
                let was_enabled = matches!(&previous, Some(ManagedPluginState::Enabled(_)));
                if was_enabled != (persisted == PluginEnabledState::Enabled) {
                    changed_ids.insert(plugin_id.clone());
                }
                let managed = match persisted {
                    PluginEnabledState::Enabled => match previous {
                        Some(ManagedPluginState::Enabled(runtime)) => {
                            ManagedPluginState::Enabled(runtime)
                        }
                        Some(ManagedPluginState::Disabled) | None => {
                            ManagedPluginState::Enabled(EnabledRuntime::Stopped)
                        }
                    },
                    PluginEnabledState::Disabled => ManagedPluginState::Disabled,
                };
                managed_by_id.insert(plugin_id.clone(), managed);
            }
            state.installed = installed;
            state.replace_managed(managed_by_id);
            changed_ids
        };
        for plugin_id in changed_ids {
            self.inner.publisher.publish_status_changed(&plugin_id);
        }

        Ok(ScanPluginsResponse {
            plugins: self.list_installed_plugins().plugins,
        })
    }
}
