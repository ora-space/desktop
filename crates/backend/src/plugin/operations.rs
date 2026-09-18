//! Public plugin use cases, including reconciliation of the process-local agent set.

use super::PluginApi;
use crate::BackendError;
use crate::agent_runtime::AgentRuntimeManager;
use crate::plugin_gateway::PluginGateway;
use crate::session_setup::SessionMcpHost;
use ora_contracts::*;
use ora_domain::PluginId;
use ora_plugin_asset::LogoAssetRoot;
use ora_utils::http::ProgressCallback;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
mod health_tests;
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
        let plugin_id = PluginId::parse(&request.plugin_id).ok();
        let response = self.host.save_configuration(request)?;
        // A save publishes a new configuration revision, so every result on the previous revision
        // is stale immediately; a member that is now complete is re-probed once.
        if let Some(plugin_id) = plugin_id {
            self.refresh_mcp_health(plugin_id);
        }
        Ok(response)
    }

    /// Executes an explicit Reset All or damaged-data recovery operation.
    pub fn reset_configuration(
        &self,
        request: ResetPluginConfigurationRequest,
    ) -> Result<ResetPluginConfigurationResponse, BackendError> {
        let plugin_id = PluginId::parse(&request.plugin_id).ok();
        let response = self.host.reset_configuration(request)?;
        // Clearing settings changes the revision or removes eligibility; either way the old health
        // result may not be presented any more.
        if let Some(plugin_id) = plugin_id {
            self.refresh_mcp_health(plugin_id);
        }
        Ok(response)
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
            // A removed member's health identity may not keep presenting once its package is gone.
            if result.is_ok() {
                for member_id in &plan.remove {
                    if let Ok(plugin_id) = PluginId::parse(member_id) {
                        self.host.mcp_health().invalidate_plugin(&plugin_id);
                    }
                }
            }
            self.agent_runtime.sync_plugin_agents();
            return result;
        }
        let plugin_id = request.plugin_id.clone();
        self.agent_runtime.suspend_plugin_agent(&plugin_id);
        let result = self.host.uninstall(request).await;
        self.agent_runtime.resume_plugin_agent(&plugin_id);
        let response = result?;
        // The package is gone, so no identity of it may keep presenting health.
        if let Ok(plugin_id) = PluginId::parse(&plugin_id) {
            self.host.mcp_health().invalidate_plugin(&plugin_id);
        }
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
        let plugin_id = PluginId::parse(&request.plugin_id).ok();
        let response = self.host.install(request).await?;
        self.probe_installed_mcp(plugin_id);
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Installs a marketplace plugin while forwarding download progress to a host callback.
    pub async fn install_with_progress(
        &self,
        request: InstallPluginRequest,
        progress: ProgressCallback,
    ) -> Result<InstallPluginResponse, BackendError> {
        let plugin_id = PluginId::parse(&request.plugin_id).ok();
        let response = self.host.install_with_progress(request, progress).await?;
        self.probe_installed_mcp(plugin_id);
        self.agent_runtime.sync_plugin_agents();
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
        self.agent_runtime.suspend_plugin_agent(&plugin_id);
        let result = self.host.update(request).await;
        self.agent_runtime.resume_plugin_agent(&plugin_id);
        let response = result?;
        // The exact package version is part of the health identity, so the replaced version's
        // result is dropped before the new version is probed once.
        if let Ok(plugin_id) = PluginId::parse(&plugin_id) {
            self.refresh_mcp_health(plugin_id);
        }
        self.agent_runtime.sync_plugin_agents();
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
        self.agent_runtime.suspend_plugin_agent(&plugin_id);
        let result = self.host.update_with_progress(request, progress).await;
        self.agent_runtime.resume_plugin_agent(&plugin_id);
        let response = result?;
        if let Ok(plugin_id) = PluginId::parse(&plugin_id) {
            self.refresh_mcp_health(plugin_id);
        }
        self.agent_runtime.sync_plugin_agents();
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
        let response = self.host.import(request).await?;
        self.probe_installed_mcp(PluginId::parse(&response.plugin_id).ok());
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Lists secret-free Host MCP health for the plugin card (no cwd) or one Session view.
    ///
    /// Only currently eligible installed members appear, and a workspace-context member without a
    /// real cwd reports `context_missing` instead of a fabricated result.
    pub fn list_mcp_health(
        &self,
        request: ListMcpHealthRequest,
    ) -> Result<ListMcpHealthResponse, BackendError> {
        let host = SessionMcpHost::from_plugin_api(self.host.clone());
        self.host.mcp_health().list(&host, &host, request)
    }

    /// Runs or joins one Host MCP probe for a currently eligible member and awaits its result.
    ///
    /// This is the user-initiated "re-detect" path: waiting is allowed because completing the probe
    /// is exactly what the request asked for, and the probe's hard timeout still bounds it.
    pub async fn probe_mcp_health(
        &self,
        request: ProbeMcpHealthRequest,
    ) -> Result<ProbeMcpHealthResponse, BackendError> {
        let host = SessionMcpHost::from_plugin_api(self.host.clone());
        self.host.mcp_health().probe(&host, &host, request).await
    }

    /// Probes one freshly installed member for the plugin card.
    ///
    /// The probe is independent of any Session selection; ineligible packages (not MCP, or still
    /// configuration-incomplete) are simply not probed.
    fn probe_installed_mcp(&self, plugin_id: Option<PluginId>) {
        let Some(plugin_id) = plugin_id else {
            return;
        };
        let host = SessionMcpHost::from_plugin_api(self.host.clone());
        self.host.mcp_health().spawn_card_probe(host, plugin_id);
    }

    /// Drops a plugin's old health result, then probes it once when it is currently eligible.
    fn refresh_mcp_health(&self, plugin_id: PluginId) {
        self.host.mcp_health().invalidate_plugin(&plugin_id);
        self.probe_installed_mcp(Some(plugin_id));
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
