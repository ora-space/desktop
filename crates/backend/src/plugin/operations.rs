//! Public plugin use cases, including reconciliation of the process-local agent set.

use super::PluginApi;
use crate::BackendError;
use crate::agent_runtime::AgentRuntimeManager;
use crate::plugin_gateway::PluginGateway;
use ora_contracts::*;
use ora_utils::http::ProgressCallback;
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
    pub fn sync_available(
        &self,
        request: SyncAvailablePluginsRequest,
    ) -> Result<SyncAvailablePluginsResponse, BackendError> {
        self.host.sync_available_plugins(request)
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
    pub async fn uninstall(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, BackendError> {
        let response = self.host.uninstall(request).await?;
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
        let response = self.host.install(request).await?;
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Installs a marketplace plugin while forwarding download progress to a host callback.
    pub async fn install_with_progress(
        &self,
        request: InstallPluginRequest,
        progress: ProgressCallback,
    ) -> Result<InstallPluginResponse, BackendError> {
        let response = self.host.install_with_progress(request, progress).await?;
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }

    /// Updates one installed marketplace plugin to the version its source publishes and
    /// reconciles the agent set afterwards.
    ///
    /// The agent set is reconciled so a replaced agent package supplies a reachable agent in this
    /// process rather than only after the next restart.
    pub async fn update(
        &self,
        request: UpdatePluginRequest,
    ) -> Result<UpdatePluginResponse, BackendError> {
        let response = self.host.update(request).await?;
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
        let response = self.host.update_with_progress(request, progress).await?;
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
        self.agent_runtime.sync_plugin_agents();
        Ok(response)
    }
}
