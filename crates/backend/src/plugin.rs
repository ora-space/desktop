use crate::app_event::AppEventPublisher;
use crate::clock::SystemClock;
use crate::error::{BackendError, ErrorClassification};
use gitlancer::{BranchName, CliGitRunner, Git};
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, DisablePluginRequest, DisablePluginResponse,
    EmptyErrorParams, EnablePluginRequest, EnablePluginResponse, InstallPluginRequest,
    InstallPluginResponse, ListAvailablePluginsRequest, ListAvailablePluginsResponse,
    ListInstalledPluginsRequest, ListInstalledPluginsResponse, PublicError, ScanPluginsRequest,
    ScanPluginsResponse, StopPluginRequest, StopPluginResponse, SyncAvailablePluginsRequest,
    SyncAvailablePluginsResponse, UninstallPluginRequest, UninstallPluginResponse,
};
use ora_db::{RepositoryPool, SqlitePluginStateRepository};
use ora_logging::{ora_info, ora_warn};
use ora_plugin_lifecycle::{
    DenoPluginRuntimeLauncher, PluginLifecycle, PluginLifecycleConfig, PluginLifecycleError,
    PluginRuntimeTimeouts,
};
use ora_plugin_manager::Installer;
use ora_plugin_registry::{
    RegistryEntry, RegistryError, RegistryIndex, RegistrySource, RegistrySync,
};
use ora_utils::http::{DownloadSource, ProxyConfig, ReqwestDownloader};
use std::path::PathBuf;

/// The marketplace repository mirrored into the local registry source checkout.
const MARKETPLACE_REPOSITORY_URL: &str = "https://github.com/ora-space/marketplace";
/// The branch tracked by the marketplace registry source.
const MARKETPLACE_REPOSITORY_BRANCH: &str = "main";

/// Groups plugin discovery and lifecycle operations behind the backend's plugin interface.
pub(crate) struct PluginApi {
    lifecycle: PluginLifecycle<
        SqlitePluginStateRepository,
        SystemClock,
        DenoPluginRuntimeLauncher,
        AppEventPublisher,
    >,
    registry_source: RegistrySource,
    registry_index_path: PathBuf,
    data_directory: PathBuf,
    installer: Installer<ReqwestDownloader>,
}

impl PluginApi {
    /// Opens plugin lifecycle state with the concrete backend adapters.
    pub(crate) fn open(
        pool: RepositoryPool,
        data_directory: PathBuf,
        deno_path: PathBuf,
        clock: SystemClock,
        publisher: AppEventPublisher,
    ) -> Result<Self, PluginLifecycleError> {
        let plugins_directory = data_directory.join("plugins");
        let registry_source = RegistrySource::new(
            MARKETPLACE_REPOSITORY_URL,
            BranchName::new(MARKETPLACE_REPOSITORY_BRANCH),
            plugins_directory
                .join("sources")
                .join("github.com")
                .join("ora-space")
                .join("marketplace"),
        );
        let registry_index_path = plugins_directory.join("cache").join("registry_index.json");
        let installer = Installer::new(ReqwestDownloader::new(ProxyConfig::default()));
        let lifecycle = PluginLifecycle::open(
            PluginLifecycleConfig {
                data_directory: data_directory.clone(),
                deno_path,
            },
            SqlitePluginStateRepository::new(pool),
            clock,
            DenoPluginRuntimeLauncher::new(PluginRuntimeTimeouts::default()),
            publisher,
        )?;

        Ok(Self {
            lifecycle,
            registry_source,
            registry_index_path,
            data_directory,
            installer,
        })
    }

    /// Returns the cached marketplace registry index, or an empty catalog when absent.
    pub(crate) fn list_available_plugins(
        &self,
        _request: ListAvailablePluginsRequest,
    ) -> Result<ListAvailablePluginsResponse, RegistryError> {
        match RegistryIndex::load(&self.registry_index_path) {
            Ok(index) => Ok(ListAvailablePluginsResponse {
                updated_at: index.updated_at(),
                plugins: index.plugins().iter().map(available_plugin).collect(),
            }),
            Err(RegistryError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ListAvailablePluginsResponse {
                    updated_at: 0,
                    plugins: Vec::new(),
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Pulls the marketplace source, rebuilds its registry index, and atomically replaces the cache.
    pub(crate) fn sync_available_plugins(
        &self,
        _request: SyncAvailablePluginsRequest,
    ) -> Result<SyncAvailablePluginsResponse, RegistryError> {
        let checkout_directory =
            RegistrySync::sync(&Git::new(CliGitRunner), &self.registry_source)?;
        let build = RegistryIndex::build(
            &checkout_directory.join("registry"),
            ora_logging::clock::now_local().unix_timestamp(),
        );
        if let Some(cache_directory) = self.registry_index_path.parent() {
            std::fs::create_dir_all(cache_directory)?;
        }
        build.index().write(&self.registry_index_path)?;
        Ok(SyncAvailablePluginsResponse {
            updated_at: build.index().updated_at(),
            plugins: build
                .index()
                .plugins()
                .iter()
                .map(available_plugin)
                .collect(),
        })
    }

    /// Returns the cached installed-plugin snapshot without rescanning the filesystem.
    pub(crate) fn list(
        &self,
        _request: ListInstalledPluginsRequest,
    ) -> ListInstalledPluginsResponse {
        self.lifecycle.list_installed_plugins()
    }

    /// Rescans packages and reconciles durable and runtime state.
    pub(crate) async fn scan(
        &self,
        request: ScanPluginsRequest,
    ) -> Result<ScanPluginsResponse, PluginLifecycleError> {
        self.lifecycle.scan_plugins(request).await
    }

    /// Persists plugin eligibility without starting its process.
    pub(crate) async fn enable(
        &self,
        request: EnablePluginRequest,
    ) -> Result<EnablePluginResponse, PluginLifecycleError> {
        self.lifecycle.enable_plugin(request).await
    }

    /// Stops a plugin when necessary before persisting ineligibility.
    pub(crate) async fn disable(
        &self,
        request: DisablePluginRequest,
    ) -> Result<DisablePluginResponse, PluginLifecycleError> {
        self.lifecycle.disable_plugin(request).await
    }

    /// Starts one enabled plugin and returns its immediate starting state.
    pub(crate) async fn activate(
        &self,
        request: ActivatePluginRequest,
    ) -> Result<ActivatePluginResponse, PluginLifecycleError> {
        self.lifecycle.activate_plugin(request).await
    }

    /// Stops one plugin process without changing durable eligibility.
    pub(crate) async fn stop(
        &self,
        request: StopPluginRequest,
    ) -> Result<StopPluginResponse, PluginLifecycleError> {
        self.lifecycle.stop_plugin(request).await
    }

    /// Stops and removes one plugin package plus its durable state.
    pub(crate) async fn uninstall(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, PluginLifecycleError> {
        self.lifecycle.uninstall_plugin(request).await
    }
    /// Installs a marketplace plugin by resolving its release manifest from the synced source and
    /// downloading, verifying, and extracting its package through the network-backed installer.
    ///
    /// The source registry is read only for the release `url`/`sha256` (the cached index carries
    /// display fields only), so this returns NotFound when the identifier is not in the checkout.
    pub(crate) async fn install(
        &self,
        request: InstallPluginRequest,
    ) -> Result<InstallPluginResponse, BackendError> {
        let registry_directory = self.registry_source.checkout_dir().join("registry");
        let manifest = RegistryIndex::resolve_manifest(&registry_directory, &request.plugin_id)
            .map_err(|error| {
                BackendError::internal("failed to resolve plugin release manifest", error)
            })?
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::NotFound,
                    PublicError::PluginNotFound(EmptyErrorParams {}),
                    "marketplace plugin was not found in the registry",
                )
            })?;
        let release_url = manifest
            .url()
            .ok_or_else(|| {
                BackendError::internal(
                    "marketplace plugin manifest is missing its release url",
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "missing release url"),
                )
            })?
            .as_url();
        ora_info!(plugin_id = %request.plugin_id, url = %release_url, "installing marketplace plugin");
        self.installer
            .install(
                &manifest,
                DownloadSource::Url(release_url.clone()),
                &self.data_directory,
            )
            .await
            .map_err(|error| BackendError::internal("failed to install plugin", error))?;
        // The installed snapshot is built once at startup, so a fresh install must re-scan for the
        // new package to appear in the installed list without restarting the backend.
        if let Err(error) = self.lifecycle.scan_plugins(ScanPluginsRequest {}).await {
            ora_warn!(plugin_id = %request.plugin_id, %error, "installed the package but failed to refresh the installed-plugin snapshot");
        }
        ora_info!(plugin_id = %request.plugin_id, "installed marketplace plugin");
        Ok(InstallPluginResponse {
            plugin_id: request.plugin_id,
        })
    }
}

/// Converts one registry entry into the frontend-facing marketplace summary.
fn available_plugin(entry: &RegistryEntry) -> ora_contracts::AvailablePlugin {
    ora_contracts::AvailablePlugin {
        id: entry.id().to_owned(),
        name: entry.name().to_owned(),
        namespace: entry.namespace().to_owned(),
        version: entry.version().to_string(),
        description: entry.description().to_owned(),
    }
}
