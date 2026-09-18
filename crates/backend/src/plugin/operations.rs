//! Public plugin use cases, including reconciliation of the process-local agent set.

use super::PluginApi;
use crate::BackendError;
use crate::agent_runtime::AgentRuntimeManager;
use crate::plugin_gateway::PluginGateway;
use ora_contracts::*;
use ora_domain::PluginId;
use ora_plugin_asset::LogoAssetRoot;
use ora_utils::http::ProgressCallback;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
mod install_tests;
#[cfg(test)]
mod tests;

/// Owns plugin operations and their runtime coordination without exposing host internals.
#[derive(Clone)]
pub struct Plugins {
    host: Arc<PluginApi>,
    agent_runtime: Arc<AgentRuntimeManager>,
}

impl Plugins {
    pub(crate) fn new(host: Arc<PluginApi>, agent_runtime: Arc<AgentRuntimeManager>) -> Self {
        Self {
            host,
            agent_runtime,
        }
    }

    /// Returns the plugin data-plane gateway the desktop surface layer drives.
    pub fn gateway(&self) -> Arc<PluginGateway> {
        Arc::new(PluginGateway::new(Arc::clone(&self.host)))
    }

    /// Returns every recorded pack installation with its reconciled member states.
    pub fn list_pack_installations(
        &self,
        _request: ListPackInstallationsRequest,
    ) -> Result<ListPackInstallationsResponse, BackendError> {
        Ok(ListPackInstallationsResponse {
            packs: self.host.list_pack_installations()?,
        })
    }

    /// Returns the ownership-aware uninstall plan for one pack id, absent for ordinary plugins.
    pub fn pack_uninstall_plan(
        &self,
        request: PackUninstallPlanRequest,
    ) -> Result<PackUninstallPlanResponse, BackendError> {
        Ok(PackUninstallPlanResponse {
            plan: self.host.pack_uninstall_plan(&request.plugin_id)?,
        })
    }

    /// Returns the cached installed-plugin snapshot without rescanning the filesystem.
    pub fn list_installed(
        &self,
        request: ListInstalledPluginsRequest,
    ) -> Result<ListInstalledPluginsResponse, BackendError> {
        Ok(self.host.list(request))
    }

    /// Returns one typed Plugin Configuration editor snapshot.
    pub fn get_configuration(
        &self,
        request: GetPluginConfigurationRequest,
    ) -> Result<GetPluginConfigurationResponse, BackendError> {
        self.host.get_configuration(request)
    }

    /// Persists one revision-checked Plugin Configuration replacement.
    pub fn save_configuration(
        &self,
        request: SavePluginConfigurationRequest,
    ) -> Result<SavePluginConfigurationResponse, BackendError> {
        self.host.save_configuration(request)
    }

    /// Executes an explicit Reset All or damaged-data recovery operation.
    pub fn reset_configuration(
        &self,
        request: ResetPluginConfigurationRequest,
    ) -> Result<ResetPluginConfigurationResponse, BackendError> {
        self.host.reset_configuration(request)
    }

    /// Returns the directory one plugin's icon candidates are served from under `root`.
    ///
    /// The icon protocol needs a root, not a file: the URL names which of the two directories it
    /// means, the plugin, the theme role and the extension, and the handler builds the candidate
    /// filename itself from those closed sets. A root that holds nothing for this id reads as
    /// `None` and the request is refused, without consulting the other root.
    pub fn logo_directory(&self, root: LogoAssetRoot, plugin_id: &PluginId) -> Option<PathBuf> {
        self.host.logo_directory(root, plugin_id)
    }

    /// Returns the cached marketplace registry index used to populate plugin discovery.
    pub fn list_available(
        &self,
        request: ListAvailablePluginsRequest,
    ) -> Result<ListAvailablePluginsResponse, BackendError> {
        self.host.list_available_plugins(request)
    }

    /// Returns every configured marketplace source in precedence order.
    pub fn list_sources(
        &self,
        request: ListMarketplaceSourcesRequest,
    ) -> Result<ListMarketplaceSourcesResponse, BackendError> {
        self.host.list_marketplace_sources(request)
    }

    /// Adds one marketplace source after validating and persisting it.
    pub fn add_source(
        &self,
        request: AddMarketplaceSourceRequest,
    ) -> Result<AddMarketplaceSourceResponse, BackendError> {
        self.host.add_marketplace_source(request)
    }

    /// Removes one marketplace source by URL after persisting the new ordering.
    pub fn delete_source(
        &self,
        request: DeleteMarketplaceSourceRequest,
    ) -> Result<DeleteMarketplaceSourceResponse, BackendError> {
        self.host.delete_marketplace_source(request)
    }

    /// Replaces the editable fields of one marketplace source after persisting them.
    pub fn update_source(
        &self,
        request: UpdateMarketplaceSourceRequest,
    ) -> Result<UpdateMarketplaceSourceResponse, BackendError> {
        self.host.update_marketplace_source(request)
    }

    /// Pulls the marketplace source and rebuilds the cache used by plugin discovery.
    ///
    /// A rebuild already in flight answers this request from the cached index instead of running
    /// a second identical one.
    pub fn sync_available(
        &self,
        request: SyncAvailablePluginsRequest,
    ) -> Result<SyncAvailablePluginsResponse, BackendError> {
        self.host.sync_available_plugins(request)
    }

    /// Admits one automatic rebuild, or reports that another rebuild already covers it.
    ///
    /// Automatic rebuilds are announced to the user before they start, so they claim admission
    /// and run as two steps: an announcement can then never describe work that was discarded.
    /// Dropping the returned value without running it releases the slot untouched.
    pub fn admit_auto_sync(&self) -> Option<AdmittedSync<'_>> {
        self.host.try_begin_rebuild().map(|slot| AdmittedSync {
            host: self.host.as_ref(),
            _slot: slot,
        })
    }

    /// Reads the README one marketplace listing publishes for its detail page.
    pub fn read_readme(
        &self,
        request: ReadPluginReadmeRequest,
    ) -> Result<ReadPluginReadmeResponse, BackendError> {
        self.host.read_plugin_readme(request)
    }

    /// Explicitly rescans packages and reconciles process-local runtime state.
    pub async fn scan(
        &self,
        request: ScanPluginsRequest,
    ) -> Result<ScanPluginsResponse, BackendError> {
        let response = self.host.scan(request).await?;
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Starts one installed plugin and returns its immediate starting state.
    pub async fn activate(
        &self,
        request: ActivatePluginRequest,
    ) -> Result<ActivatePluginResponse, BackendError> {
        self.host
            .activate(request)
            .await
            .map_err(BackendError::from)
    }

    /// Stops one plugin process while leaving the installed plugin available.
    pub async fn stop(
        &self,
        request: StopPluginRequest,
    ) -> Result<StopPluginResponse, BackendError> {
        self.host.stop(request).await.map_err(BackendError::from)
    }

    /// Stops and removes one plugin package plus its process-local state.
    ///
    /// A pack id carries an ownership journal instead of a package: its removable members are
    /// uninstalled through the ordinary single-plugin chain under an ownership-aware plan, and
    /// the journal releases one relationship at a time. The agent supervisor is suspended for
    /// every removed member so its respawn loop cannot race the package removal.
    pub async fn uninstall(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, BackendError> {
        if let Some(plan) = self.host.pack_uninstall_plan(&request.plugin_id)? {
            let member_ids = plan
                .remove
                .iter()
                .chain(plan.preserve.iter().map(|entry| &entry.member_id))
                .chain(plan.already_missing.iter())
                .cloned()
                .collect::<Vec<_>>();
            for member_id in &plan.remove {
                self.agent_runtime.suspend_plugin_agent(member_id);
            }
            let result = self
                .host
                .uninstall_pack(&request.plugin_id, request.data_disposition)
                .await;
            for member_id in &member_ids {
                self.agent_runtime.resume_plugin_agent(member_id);
            }
            self.agent_runtime.sync_plugin_agents();
            return result;
        }
        let plugin_id = request.plugin_id.clone();
        self.agent_runtime.suspend_plugin_agent(&plugin_id);
        // Deinitialization runs before the package is removed, and only when the user authorized
        // it: the command is the package's own program, and it can only run while the package is
        // still on disk. A Hook whose command fails still uninstalls (D8).
        if request.hook_execution_acknowledged {
            self.host.deinitialize_installed_hook(&plugin_id).await;
        }
        let result = self.host.uninstall(request).await;
        self.agent_runtime.resume_plugin_agent(&plugin_id);
        let response = result?;
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Installs a marketplace plugin by resolving its release manifest from the synced source and
    /// downloading, verifying, and extracting its package through the network-backed installer.
    ///
    /// The agent set is reconciled afterwards so the newly installed package supplies a reachable
    /// agent in this process rather than only after the next restart.
    pub async fn install(
        &self,
        request: InstallPluginRequest,
    ) -> Result<InstallPluginResponse, BackendError> {
        let acknowledged = request.hook_execution_acknowledged;
        let response = self.host.install(request).await?;
        self.agent_runtime.sync_plugin_agents();
        self.initialize_hook_landed(&response.plugin_id, &response.outcome, acknowledged)
            .await;
        Ok(response)
    }

    /// Installs a marketplace plugin while forwarding download progress to a host callback.
    pub async fn install_with_progress(
        &self,
        request: InstallPluginRequest,
        progress: ProgressCallback,
    ) -> Result<InstallPluginResponse, BackendError> {
        let acknowledged = request.hook_execution_acknowledged;
        let response = self.host.install_with_progress(request, progress).await?;
        self.agent_runtime.sync_plugin_agents();
        self.initialize_hook_landed(&response.plugin_id, &response.outcome, acknowledged)
            .await;
        Ok(response)
    }

    /// Updates one installed marketplace plugin to the version its source publishes and
    /// reconciles the agent set afterwards.
    ///
    /// The agent set is reconciled so a replaced agent package supplies a reachable agent in this
    /// process rather than only after the next restart. The agent supervisor is suspended for the
    /// operation's duration: its respawn loop would otherwise re-attach to the version directory
    /// the update retires, and on Windows that directory handle blocks the replacement.
    pub async fn update(
        &self,
        request: UpdatePluginRequest,
    ) -> Result<UpdatePluginResponse, BackendError> {
        let plugin_id = request.plugin_id.clone();
        let acknowledged = request.hook_execution_acknowledged;
        self.agent_runtime.suspend_plugin_agent(&plugin_id);
        let result = self.host.update(request).await;
        self.agent_runtime.resume_plugin_agent(&plugin_id);
        let response = result?;
        self.agent_runtime.sync_plugin_agents();
        // Every update re-runs `init`, because only the tool knows whether the version it is
        // replacing needs its Agent configuration migrated (D2).
        if acknowledged {
            self.host.initialize_installed_hook(&plugin_id).await;
        }
        Ok(response)
    }

    /// Updates one installed marketplace plugin while forwarding download progress to a host
    /// callback, reconciling the agent set on the same terms as an unobserved update.
    pub async fn update_with_progress(
        &self,
        request: UpdatePluginRequest,
        progress: ProgressCallback,
    ) -> Result<UpdatePluginResponse, BackendError> {
        let plugin_id = request.plugin_id.clone();
        let acknowledged = request.hook_execution_acknowledged;
        self.agent_runtime.suspend_plugin_agent(&plugin_id);
        let result = self.host.update_with_progress(request, progress).await;
        self.agent_runtime.resume_plugin_agent(&plugin_id);
        let response = result?;
        self.agent_runtime.sync_plugin_agents();
        if acknowledged {
            self.host.initialize_installed_hook(&plugin_id).await;
        }
        Ok(response)
    }

    /// Imports one local release archive and reconciles the agent set afterwards.
    ///
    /// The agent set is reconciled so the imported package supplies a reachable agent in this
    /// process rather than only after the next restart.
    pub async fn import(
        &self,
        request: ImportPluginRequest,
    ) -> Result<ImportPluginResponse, BackendError> {
        let acknowledged = request.hook_execution_acknowledged;
        let response = self.host.import(request).await?;
        self.agent_runtime.sync_plugin_agents();
        self.initialize_hook_landed(&response.plugin_id, &response.outcome, acknowledged)
            .await;
        Ok(response)
    }

    /// Lists this session's Hook lifecycle results for the settings surface to merge by plugin.
    ///
    /// Reading the store cannot fail; the result type keeps this operation shaped like every other
    /// one the Desktop surface drives, so the transport does not need a second response mode for
    /// an operation that happens to be infallible.
    pub fn list_hook_lifecycle_reports(
        &self,
        _request: ListHookLifecycleReportsRequest,
    ) -> Result<ListHookLifecycleReportsResponse, BackendError> {
        Ok(ListHookLifecycleReportsResponse {
            reports: self.host.hook_lifecycle_reports(),
        })
    }

    /// Runs one user-requested `init` for an installed Hook package.
    ///
    /// This is the retry path for a failed `init` and the only way a pack-installed Hook member
    /// is initialized: the pack landed the package without running anything, so the user
    /// authorizes that member on its own afterwards (D2).
    pub async fn initialize_hook(
        &self,
        request: InitializeHookRequest,
    ) -> Result<InitializeHookResponse, BackendError> {
        self.host.initialize_hook(request).await
    }

    /// Runs `init` for a Hook that a single-plugin install or import just landed.
    ///
    /// Pack members are excluded by the outcome rather than by the caller: a pack reports
    /// `PackInstalled`, so nothing the pack brought in is executed, which is exactly the
    /// authorization boundary the decision draws.
    async fn initialize_hook_landed(
        &self,
        plugin_id: &str,
        outcome: &InstallOutcome,
        acknowledged: bool,
    ) {
        if acknowledged && matches!(outcome, InstallOutcome::Installed) {
            self.host.initialize_installed_hook(plugin_id).await;
        }
    }
}

/// One admitted marketplace rebuild, held from admission until it runs.
///
/// Holding this value *is* holding the rebuild slot, so no other rebuild can start while it is
/// alive. Dropping it without calling [`AdmittedSync::run`] releases the slot and leaves the
/// cached index untouched.
pub struct AdmittedSync<'a> {
    host: &'a PluginApi,
    _slot: std::sync::MutexGuard<'a, ()>,
}

impl AdmittedSync<'_> {
    /// Pulls every configured source and atomically replaces the cached registry index.
    pub fn run(self) -> Result<SyncAvailablePluginsResponse, BackendError> {
        self.host.rebuild_registry_index()
    }
}
