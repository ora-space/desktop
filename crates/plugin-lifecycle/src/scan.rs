use super::{
    ManagedPluginState, PluginLifecycle, PluginLifecycleError, PluginNotificationSink,
    PluginRuntime, PluginRuntimeLauncher, PluginStatusPublisher,
    has_valid_configuration_declaration,
};
use ora_contracts::{ScanPluginsRequest, ScanPluginsResponse};
use ora_logging::ora_warn;
use ora_plugin_manager::PluginManager;
use std::collections::{BTreeMap, BTreeSet};

impl<RuntimeLauncher, StatusPublisher, NotificationSink>
    PluginLifecycle<RuntimeLauncher, StatusPublisher, NotificationSink>
where
    RuntimeLauncher: PluginRuntimeLauncher,
    StatusPublisher: PluginStatusPublisher,
    NotificationSink: PluginNotificationSink,
{
    /// Rebuilds the installed snapshot while retaining live runtime state when possible.
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
                    Some(ManagedPluginState::Running { runtime, .. }) => {
                        Some((plugin_id.clone(), runtime.clone()))
                    }
                    Some(ManagedPluginState::Stopped)
                    | Some(ManagedPluginState::Starting { .. })
                    | Some(ManagedPluginState::Failed { .. })
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

        let invalid_runtimes = {
            let state = self.read_state();
            installed_ids
                .iter()
                .filter_map(|plugin_id| {
                    let valid = installed_by_id
                        .get(plugin_id)
                        .is_some_and(|plugin| has_valid_configuration_declaration(plugin));
                    if valid {
                        return None;
                    }

                    match state.managed(plugin_id) {
                        Some(ManagedPluginState::Running { runtime, .. }) => {
                            Some((plugin_id.clone(), runtime.clone()))
                        }
                        Some(ManagedPluginState::Stopped)
                        | Some(ManagedPluginState::Starting { .. })
                        | Some(ManagedPluginState::Failed { .. })
                        | None => None,
                    }
                })
                .collect::<Vec<_>>()
        };
        for (plugin_id, runtime) in invalid_runtimes {
            runtime
                .stop()
                .await
                .map_err(|source| PluginLifecycleError::RuntimeStop {
                    plugin_id: plugin_id.to_string(),
                    source,
                })?;
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
                let valid = installed_by_id
                    .get(plugin_id)
                    .is_some_and(|plugin| has_valid_configuration_declaration(plugin));
                let was_active = matches!(
                    &previous,
                    Some(ManagedPluginState::Starting { .. } | ManagedPluginState::Running { .. })
                );
                if was_active && !valid {
                    changed_ids.insert(plugin_id.clone());
                }
                let managed = if valid {
                    match previous {
                        Some(managed) => managed,
                        None => ManagedPluginState::Stopped,
                    }
                } else {
                    ManagedPluginState::Stopped
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
