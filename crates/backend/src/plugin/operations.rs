//! Public plugin use cases, including reconciliation of the process-local agent set.

use super::{ImportedPlugin, InstalledPackage, PluginApi, UpdatedPackage};
use crate::BackendError;
use crate::agent_runtime::AgentRuntimeManager;
use crate::error::ErrorClassification;
use crate::plugin_gateway::PluginGateway;
use crate::session_setup::SessionMcpHost;
use crate::workflow::{MarketplaceWorkflowImport, WorkflowImport};
use ora_application::{MarketplaceWorkflowDocument, WorkflowDocumentSource};
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

/// Renders one lifecycle level state in the closed wire vocabulary.
fn plugin_log_level_response(
    plugin_id: String,
    state: ora_plugin_lifecycle::PluginLogLevelState,
) -> PluginLogLevelResponse {
    PluginLogLevelResponse {
        plugin_id,
        level: match state.level {
            ora_logging::LogLevel::Trace => RuntimeLogLevel::Trace,
            ora_logging::LogLevel::Debug => RuntimeLogLevel::Debug,
            ora_logging::LogLevel::Info => RuntimeLogLevel::Info,
            ora_logging::LogLevel::Warn => RuntimeLogLevel::Warn,
            ora_logging::LogLevel::Error => RuntimeLogLevel::Error,
        },
        configured: state.configured,
    }
}

/// Converts a validated contract level into the shared logging vocabulary.
fn internal_log_level(level: RuntimeLogLevel) -> ora_logging::LogLevel {
    match level {
        RuntimeLogLevel::Trace => ora_logging::LogLevel::Trace,
        RuntimeLogLevel::Debug => ora_logging::LogLevel::Debug,
        RuntimeLogLevel::Info => ora_logging::LogLevel::Info,
        RuntimeLogLevel::Warn => ora_logging::LogLevel::Warn,
        RuntimeLogLevel::Error => ora_logging::LogLevel::Error,
    }
}

/// Owns plugin operations and their runtime coordination without exposing host internals.
#[derive(Clone)]
pub struct Plugins {
    host: Arc<PluginApi>,
    agent_runtime: Arc<AgentRuntimeManager>,
    workflow_import: Arc<WorkflowImport>,
    marketplace_workflow_import: Arc<MarketplaceWorkflowImport>,
}

impl Plugins {
    pub(crate) fn new(
        host: Arc<PluginApi>,
        agent_runtime: Arc<AgentRuntimeManager>,
        workflow_import: Arc<WorkflowImport>,
        marketplace_workflow_import: Arc<MarketplaceWorkflowImport>,
    ) -> Self {
        Self {
            host,
            agent_runtime,
            workflow_import,
            marketplace_workflow_import,
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
        // Deinitialization runs before the package is removed, and only when the user authorized
        // it: the command is the package's own program, and it can only run while the package is
        // still on disk. A Hook whose command fails still uninstalls (D8).
        if request.hook_execution_acknowledged {
            self.host.deinitialize_installed_hook(&plugin_id).await;
        }
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

    /// Resolves the active plugin log file the desktop export copies for one installed plugin.
    pub fn log_file_path(&self, plugin_id: &str) -> Result<PathBuf, BackendError> {
        Ok(self.host.lifecycle.plugin_log_file(plugin_id)?)
    }

    /// Reads the effective host-owned log level of one plugin identity.
    pub fn get_log_level(
        &self,
        request: GetPluginLogLevelRequest,
    ) -> Result<PluginLogLevelResponse, BackendError> {
        let state = self.host.lifecycle.plugin_log_level(&request.plugin_id)?;
        Ok(plugin_log_level_response(request.plugin_id, state))
    }

    /// Persists a plugin's log level and applies it to its running generation, if any.
    ///
    /// The lifecycle persists before it applies, so a returned error means nothing changed and
    /// the frontend must not show the requested level as current. The call is ordered against
    /// uninstalls of the same plugin by the lifecycle's per-plugin operation lock.
    pub async fn set_log_level(
        &self,
        request: SetPluginLogLevelRequest,
    ) -> Result<PluginLogLevelResponse, BackendError> {
        let state = self
            .host
            .lifecycle
            .set_plugin_log_level(&request.plugin_id, internal_log_level(request.level))
            .await?;
        Ok(plugin_log_level_response(request.plugin_id, state))
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
        let plugin_id = PluginId::parse(&request.plugin_id).ok();
        let installed = self.host.install(request).await?;
        self.probe_installed_mcp(plugin_id);
        self.agent_runtime.sync_plugin_agents();
        self.initialize_hook_landed(&installed.plugin_id, &installed.outcome, acknowledged)
            .await;
        Ok(self.installed_package_response(installed))
    }

    /// Installs a marketplace plugin while forwarding download progress to a host callback.
    pub async fn install_with_progress(
        &self,
        request: InstallPluginRequest,
        progress: ProgressCallback,
    ) -> Result<InstallPluginResponse, BackendError> {
        let acknowledged = request.hook_execution_acknowledged;
        let plugin_id = PluginId::parse(&request.plugin_id).ok();
        let installed = self.host.install_with_progress(request, progress).await?;
        self.probe_installed_mcp(plugin_id);
        self.agent_runtime.sync_plugin_agents();
        self.initialize_hook_landed(&installed.plugin_id, &installed.outcome, acknowledged)
            .await;
        Ok(self.installed_package_response(installed))
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
        // The exact package version is part of the health identity, so the replaced version's
        // result is dropped before the new version is probed once.
        if let Ok(plugin_id) = PluginId::parse(&plugin_id) {
            self.refresh_mcp_health(plugin_id);
        }
        self.agent_runtime.sync_plugin_agents();
        // Every update re-runs `init`, because only the tool knows whether the version it is
        // replacing needs its Agent configuration migrated (D2).
        if acknowledged {
            self.host.initialize_installed_hook(&plugin_id).await;
        }
        Ok(self.updated_package_response(response))
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
        if let Ok(plugin_id) = PluginId::parse(&plugin_id) {
            self.refresh_mcp_health(plugin_id);
        }
        self.agent_runtime.sync_plugin_agents();
        if acknowledged {
            self.host.initialize_installed_hook(&plugin_id).await;
        }
        Ok(self.updated_package_response(response))
    }

    /// Imports one local release archive, its workflow documents, and reconciles the agent set.
    ///
    /// The agent set is reconciled so the imported package supplies a reachable agent in this
    /// process rather than only after the next restart, and the package's MCP members are probed
    /// so their health reaches the plugin card without waiting for one. Workflow documents import
    /// only once the package is committed: a package that fails to install therefore creates no
    /// workflows, while a document that fails to import never removes the package that carried it.
    pub async fn import(
        &self,
        request: ImportPluginRequest,
    ) -> Result<ImportPluginResponse, BackendError> {
        let acknowledged = request.hook_execution_acknowledged;
        let ImportedPlugin {
            plugin_id,
            outcome,
            workflow_documents,
        } = self.host.import(request).await?;
        self.probe_installed_mcp(PluginId::parse(&plugin_id).ok());
        self.agent_runtime.sync_plugin_agents();
        self.initialize_hook_landed(&plugin_id, &outcome, acknowledged)
            .await;
        let workflows = self.workflow_import.handle(workflow_documents);
        Ok(ImportPluginResponse {
            plugin_id,
            outcome,
            workflows,
        })
    }

    /// Builds an install response that reports the installed package's document outcomes.
    ///
    /// The installation outcome and the per-document outcomes stay separate on purpose: a
    /// document that could not be imported must never turn a successful installation into a failed
    /// one, and a successful installation must never claim every document was imported.
    fn installed_package_response(&self, installed: InstalledPackage) -> InstallPluginResponse {
        let workflows = self
            .marketplace_workflow_import
            .handle(marketplace_documents(installed.workflow_documents));
        InstallPluginResponse {
            plugin_id: installed.plugin_id,
            outcome: installed.outcome,
            workflows,
        }
    }

    /// Builds an update response that reports the new version's document outcomes.
    fn updated_package_response(&self, updated: UpdatedPackage) -> UpdatePluginResponse {
        let workflows = self
            .marketplace_workflow_import
            .handle(marketplace_documents(updated.workflow_documents));
        UpdatePluginResponse {
            plugin_id: updated.plugin_id,
            workflows,
        }
    }

    /// Retries the workflow document import of one installed plugin.
    ///
    /// This is the explicit action on an installed plugin card, not a second installation: it
    /// reconsiders only the document paths without a terminal source record, so already imported
    /// and conflict-skipped paths stay untouched and no package is downloaded or replaced. A pack's
    /// workflow member is retried through its own card, which is why the request names a member.
    pub fn retry_workflow_import(
        &self,
        request: RetryWorkflowImportRequest,
    ) -> Result<RetryWorkflowImportResponse, BackendError> {
        let installed = self
            .host
            .list(ListInstalledPluginsRequest {})
            .plugins
            .iter()
            .any(|plugin| plugin.id == request.plugin_id);
        if !installed {
            return Err(BackendError::new(
                ErrorClassification::NotFound,
                PublicError::PluginNotFound(EmptyErrorParams {}),
                "the plugin to retry workflow import for is not installed",
            ));
        }
        let documents = self
            .host
            .contributed_workflow_documents(std::slice::from_ref(&request.plugin_id))?;
        Ok(RetryWorkflowImportResponse {
            plugin_id: request.plugin_id,
            workflows: self
                .marketplace_workflow_import
                .handle(marketplace_documents(documents)),
        })
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

/// Adapts the plugin layer's attributed documents to the marketplace import use case's input.
///
/// The plugin layer reports the member's canonical id in two segments because that is the shape its
/// own identity has; the import use case keeps the same two segments because they are the first two
/// thirds of its durable source key.
fn marketplace_documents(
    documents: Vec<super::ContributedWorkflowDocument>,
) -> Vec<MarketplaceWorkflowDocument> {
    documents
        .into_iter()
        .map(|contributed| MarketplaceWorkflowDocument {
            source: WorkflowDocumentSource {
                namespace: contributed.namespace,
                identifier: contributed.identifier,
                document_path: contributed.document.source_file,
            },
            contents: contributed.document.contents,
        })
        .collect()
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
