use crate::app_event::AppEventPublisher;
use crate::clock::SystemClock;
use crate::effect_worker::EffectWorkerHandle;
use crate::error::{BackendError, ErrorClassification};
use crate::marketplace_sources::{MarketplaceSourceStore, MarketplaceSourceStoreError};
use crate::proxy;
use crate::user_config::UserConfigApi;
use gitlancer::{CliGitRunner, Git};
use ora_application::Clock;
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, AddMarketplaceSourceRequest,
    AddMarketplaceSourceResponse, DeleteMarketplaceSourceRequest, DeleteMarketplaceSourceResponse,
    EmptyErrorParams, ImportPluginRequest, ImportPluginResponse, InstallPluginRequest,
    InstallPluginResponse, ListAvailablePluginsRequest, ListAvailablePluginsResponse,
    ListInstalledPluginsRequest, ListInstalledPluginsResponse, ListMarketplaceSourcesRequest,
    ListMarketplaceSourcesResponse, PublicError, ScanPluginsRequest, ScanPluginsResponse,
    StopPluginRequest, StopPluginResponse, SyncAvailablePluginsRequest,
    SyncAvailablePluginsResponse, UninstallPluginRequest, UninstallPluginResponse,
    UpdateMarketplaceSourceRequest, UpdateMarketplaceSourceResponse, UpdatePluginRequest,
    UpdatePluginResponse,
};
use ora_db::{
    PluginSkillProjection, RepositoryPool, SqliteEffectRepository,
    SqlitePluginMarketplaceSourceRepository, SqliteSkillRepository, SqliteWorkspaceRepository,
};
use ora_domain::{PluginId, WorkspaceLocation};
use ora_effect::{Digest, FilesystemSkillSurface, SurfaceDescriptorSet};
use ora_logging::{ora_debug, ora_info, ora_warn};
use ora_plugin_config::ConfigurationService;
use ora_plugin_lifecycle::{
    ConnectionError, DenoPluginRuntime, DenoPluginRuntimeLauncher, InboundNotification,
    PluginGenerationKey, PluginGenerationLease, PluginLifecycle, PluginLifecycleConfig,
    PluginLifecycleError, PluginNotificationSink, PluginRuntimeTimeouts,
};
use ora_plugin_manager::{Installer, PluginContribution, PluginManager};
use ora_plugin_manifest::PluginManifest;
use ora_plugin_registry::{RegistryEntry, RegistryError, RegistryIndex, RegistrySync};
use ora_utils::http::{DownloadSource, ProxyConfig, ReqwestDownloader};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// The concrete lifecycle composition the backend runs.
pub(crate) type BackendPluginLifecycle =
    PluginLifecycle<DenoPluginRuntimeLauncher, AppEventPublisher, BroadcastNotificationSink>;

/// Bounded fan-out buffer between the lifecycle's per-process pumps and their consumers.
///
/// A slow broadcast subscriber may lag and lose the oldest notifications, which is acceptable
/// because the broadcast path carries only best-effort traffic (`ora/ui/push`); the alternative,
/// blocking the pump, would stall every other message of that plugin process. Traffic that must
/// not be dropped goes through a per-generation tap instead.
const NOTIFICATION_CHANNEL_CAPACITY: usize = 256;

/// How long an agent attachment waits for a plugin launch to settle.
///
/// This exceeds the runtime's ready timeout so a slow handshake is reported by the launch itself
/// as a failed runtime, with its reason, rather than as an opaque attach timeout.
const AGENT_ATTACH_WAIT: Duration = Duration::from_secs(15);

/// Forwards plugin-originated notifications to every subscriber without blocking the pump.
///
/// Subscribers are attached after the backend is built (the desktop surface host is one), which
/// is why the sink owns a broadcast sender instead of a fixed consumer. Consumers that cannot
/// tolerate a dropped frame, such as an agent connection reading ACP, tap one process generation
/// through an unbounded channel: the process runtime already buffers unboundedly, so the tap adds
/// no new loss point, and the pump never waits on either path.
#[derive(Clone, Debug)]
pub(crate) struct BroadcastNotificationSink {
    sender: broadcast::Sender<InboundNotification>,
    taps: Arc<Mutex<Vec<NotificationTap>>>,
}

/// One lossless subscription to the notifications of a single process generation.
#[derive(Debug)]
struct NotificationTap {
    plugin_id: PluginId,
    generation: PluginGenerationKey,
    sender: mpsc::UnboundedSender<InboundNotification>,
}

impl BroadcastNotificationSink {
    fn new() -> Self {
        let (sender, _) = broadcast::channel(NOTIFICATION_CHANNEL_CAPACITY);
        Self {
            sender,
            taps: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Opens a new receiver that sees every notification sent from now on.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<InboundNotification> {
        self.sender.subscribe()
    }

    /// Opens a lossless receiver of every notification `generation` of `plugin_id` emits from now
    /// on. The tap is released when its receiver is dropped.
    fn tap(
        &self,
        plugin_id: &PluginId,
        generation: PluginGenerationKey,
    ) -> mpsc::UnboundedReceiver<InboundNotification> {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.taps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(NotificationTap {
                plugin_id: plugin_id.clone(),
                generation,
                sender,
            });
        receiver
    }
}

impl PluginNotificationSink for BroadcastNotificationSink {
    /// Publishes the notification to the broadcast subscribers and to the taps of its generation.
    ///
    /// With no broadcast subscriber it is dropped there, which is logged at debug level so an
    /// unexpectedly silent plugin stays diagnosable; taps whose receiver went away are pruned as
    /// they are encountered.
    fn on_notification(&self, notification: InboundNotification) {
        if self.sender.send(notification.clone()).is_err() {
            ora_debug!(
                message = "plugin notification dropped without subscriber",
                plugin_id = %notification.plugin_id,
                generation = notification.generation.0,
                method = %notification.method,
            );
        }
        self.taps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|tap| {
                if tap.plugin_id != notification.plugin_id
                    || tap.generation != notification.generation
                {
                    return !tap.sender.is_closed();
                }
                tap.sender.send(notification.clone()).is_ok()
            });
    }
}

/// A running agent plugin process together with a lossless stream of what it emits.
///
/// The process stays owned by the lifecycle: the connection is pinned to one generation and the
/// notification stream covers exactly that generation, so a restarted plugin can never leak frames
/// into a connection that belonged to its predecessor.
pub(crate) struct AgentPluginAttachment {
    pub connection: PluginGenerationLease<DenoPluginRuntime>,
    pub notifications: mpsc::UnboundedReceiver<InboundNotification>,
}

/// Groups plugin discovery and lifecycle operations behind the backend's plugin interface.
pub(crate) struct PluginApi {
    pub(crate) lifecycle: BackendPluginLifecycle,
    marketplace_sources: MarketplaceSourceStore,
    user_config: Arc<UserConfigApi>,
    registry_index_path: PathBuf,
    data_directory: PathBuf,
    installer: Installer<ReqwestDownloader>,
    notifications: BroadcastNotificationSink,
    pub(crate) configuration: ConfigurationService,
    skill_repository: SqliteSkillRepository,
    effect_repository: SqliteEffectRepository,
    workspace_repository: SqliteWorkspaceRepository,
    agent_effect_surfaces: Mutex<BTreeMap<PluginId, Vec<FilesystemSkillSurface>>>,
    /// Set once the Effect worker exists, which is after this API the worker itself borrows.
    ///
    /// Its absence only costs latency: a declaration change is already durable before the wake
    /// would fire, so the worker's periodic scan still converges the surface.
    effect_reconcile: OnceLock<EffectWorkerHandle>,
    clock: SystemClock,
}

impl PluginApi {
    /// Opens plugin lifecycle state with the concrete backend adapters.
    pub(crate) fn open(
        pool: RepositoryPool,
        data_directory: PathBuf,
        deno_path: PathBuf,
        clock: SystemClock,
        publisher: AppEventPublisher,
        user_config: Arc<UserConfigApi>,
    ) -> Result<Self, BackendError> {
        let plugins_directory = data_directory.join("plugins");
        let marketplace_sources = MarketplaceSourceStore::open(
            SqlitePluginMarketplaceSourceRepository::new(pool.clone()),
            &data_directory,
            clock.now_timestamp_millis(),
        )
        .map_err(|error| {
            BackendError::internal(
                "failed to load configured plugin marketplace sources",
                error,
            )
        })?;
        let registry_index_path = plugins_directory.join("cache").join("registry_index.json");
        let installer = Installer::new(ReqwestDownloader::new(ProxyConfig::default()));
        let notifications = BroadcastNotificationSink::new();
        let configuration = ConfigurationService::new(data_directory.clone());
        let lifecycle = PluginLifecycle::open(
            PluginLifecycleConfig {
                data_directory: data_directory.clone(),
                deno_path,
            },
            DenoPluginRuntimeLauncher::new(PluginRuntimeTimeouts::default()),
            publisher,
            notifications.clone(),
        )
        .map_err(BackendError::from)?;

        Ok(Self {
            lifecycle,
            marketplace_sources,
            user_config,
            registry_index_path,
            data_directory,
            installer,
            notifications,
            configuration,
            skill_repository: SqliteSkillRepository::new(pool.clone()),
            effect_repository: SqliteEffectRepository::new(pool.clone()),
            workspace_repository: SqliteWorkspaceRepository::new(pool),
            agent_effect_surfaces: Mutex::new(BTreeMap::new()),
            effect_reconcile: OnceLock::new(),
            clock,
        })
    }

    /// Connects the Effect worker's wake handle once it exists.
    ///
    /// The worker borrows this API, so it cannot be built before it; wiring the wake afterwards
    /// keeps that one-way dependency instead of making the two constructors mutually recursive.
    pub(crate) fn set_effect_reconcile(&self, handle: EffectWorkerHandle) {
        let _ = self.effect_reconcile.set(handle);
    }

    /// Rebuilds catalog projections for every Skill plugin already installed on disk.
    pub(crate) fn sync_installed_skills(&self) -> Result<(), BackendError> {
        let manager = PluginManager::discover(&self.data_directory);
        for plugin in manager.installed_plugins() {
            if matches!(plugin.contributes, PluginContribution::Skill(_)) {
                self.persist_discovered_plugin_skills(plugin)?;
            }
        }
        Ok(())
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

    /// Returns the user-configured marketplace source repositories in precedence order.
    pub(crate) fn list_marketplace_sources(
        &self,
        _request: ListMarketplaceSourcesRequest,
    ) -> Result<ListMarketplaceSourcesResponse, BackendError> {
        Ok(ListMarketplaceSourcesResponse {
            sources: self
                .marketplace_sources
                .list()
                .map_err(map_marketplace_source_error)?,
        })
    }

    /// Adds and persists one marketplace source after validating its URL and branch.
    pub(crate) fn add_marketplace_source(
        &self,
        request: AddMarketplaceSourceRequest,
    ) -> Result<AddMarketplaceSourceResponse, BackendError> {
        let sources = self
            .marketplace_sources
            .add(
                ora_contracts::MarketplaceSource {
                    url: request.url,
                    branch: request.branch,
                    use_proxy: request.use_proxy,
                },
                self.clock.now_timestamp_millis(),
            )
            .map_err(map_marketplace_source_error)?;
        Ok(AddMarketplaceSourceResponse { sources })
    }

    /// Removes and persists one marketplace source by URL.
    pub(crate) fn delete_marketplace_source(
        &self,
        request: DeleteMarketplaceSourceRequest,
    ) -> Result<DeleteMarketplaceSourceResponse, BackendError> {
        let sources = self
            .marketplace_sources
            .delete(&request.url)
            .map_err(map_marketplace_source_error)?;
        Ok(DeleteMarketplaceSourceResponse { sources })
    }

    /// Changes only one marketplace source\u2019s proxy policy and returns the authoritative list.
    pub(crate) fn update_marketplace_source(
        &self,
        request: UpdateMarketplaceSourceRequest,
    ) -> Result<UpdateMarketplaceSourceResponse, BackendError> {
        let sources = self
            .marketplace_sources
            .set_use_proxy(
                &request.url,
                request.use_proxy,
                self.clock.now_timestamp_millis(),
            )
            .map_err(map_marketplace_source_error)?;
        Ok(UpdateMarketplaceSourceResponse { sources })
    }

    /// Pulls every marketplace source, merges their registry indexes, and atomically replaces the
    /// cache.
    pub(crate) fn sync_available_plugins(
        &self,
        _request: SyncAvailablePluginsRequest,
    ) -> Result<SyncAvailablePluginsResponse, BackendError> {
        let git = Git::new(CliGitRunner);
        let registry_sources = self.prepared_registry_sources()?;
        let mut registry_dirs: Vec<PathBuf> = Vec::with_capacity(registry_sources.len());
        for (source, _) in &registry_sources {
            let checkout_directory = RegistrySync::sync(&git, source)
                .map_err(|error| BackendError::internal("failed to sync plugin registry", error))?;
            registry_dirs.push(checkout_directory.join("registry"));
        }
        let registry_dir_refs: Vec<&Path> = registry_dirs.iter().map(PathBuf::as_path).collect();
        let build = RegistryIndex::build_all(
            &registry_dir_refs,
            ora_logging::clock::now_local().unix_timestamp(),
        );
        if let Some(cache_directory) = self.registry_index_path.parent() {
            std::fs::create_dir_all(cache_directory).map_err(|error| {
                BackendError::internal("failed to create registry cache directory", error)
            })?;
        }
        build
            .index()
            .write(&self.registry_index_path)
            .map_err(|error| {
                BackendError::internal("failed to write plugin registry index", error)
            })?;
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

    /// Binds every configured marketplace source to a registry checkout, applying proxy policy.
    fn prepared_registry_sources(
        &self,
    ) -> Result<Vec<(ora_plugin_registry::RegistrySource, bool)>, BackendError> {
        let configured = self
            .marketplace_sources
            .list()
            .map_err(map_marketplace_source_error)?;
        let proxy_settings = self.user_config.network_proxy_settings()?;
        let mut registry_sources = Vec::with_capacity(configured.len());

        for source in &configured {
            let mut registry_source = self
                .marketplace_sources
                .registry_source(source)
                .map_err(map_marketplace_source_error)?;
            if source.use_proxy {
                let git_env = proxy::git_proxy_env(proxy_settings.as_ref())?.ok_or_else(|| {
                    BackendError::invalid_proxy_settings(
                        "a marketplace source uses the proxy but no proxy is configured",
                    )
                })?;
                registry_source = registry_source.with_git_env(git_env);
            }
            registry_sources.push((registry_source, source.use_proxy));
        }

        Ok(registry_sources)
    }

    /// Exposes the lifecycle to the gateway that serves desktop surfaces.
    pub(crate) fn lifecycle(&self) -> &BackendPluginLifecycle {
        &self.lifecycle
    }

    /// Opens a receiver of every notification running plugin processes emit from now on.
    pub(crate) fn subscribe_notifications(&self) -> broadcast::Receiver<InboundNotification> {
        self.notifications.subscribe()
    }

    /// Returns the cached installed-plugin snapshot without rescanning the filesystem.
    pub(crate) fn list(
        &self,
        _request: ListInstalledPluginsRequest,
    ) -> ListInstalledPluginsResponse {
        self.lifecycle.list_installed_plugins()
    }

    /// Rescans packages and reconciles process-local runtime state.
    pub(crate) async fn scan(
        &self,
        request: ScanPluginsRequest,
    ) -> Result<ScanPluginsResponse, PluginLifecycleError> {
        self.lifecycle.scan_plugins(request).await
    }

    /// Starts one installed plugin and returns its immediate starting state.
    pub(crate) async fn activate(
        &self,
        request: ActivatePluginRequest,
    ) -> Result<ActivatePluginResponse, PluginLifecycleError> {
        self.lifecycle.activate_plugin(request).await
    }

    /// Returns a connection to a running plugin plus a lossless stream of its notifications,
    /// starting the installed plugin when it is stopped.
    ///
    /// This is the single seam through which the agent runtime reaches a plugin process. The
    /// process stays owned by the lifecycle, so an agent connection can never leave one running
    /// that the settings surface reports as stopped. The tap is opened after the connection is
    /// pinned so it observes exactly that generation; frames the plugin emitted before the agent
    /// was started belong to no connection and are discarded by the caller.
    pub(crate) async fn attach_agent(
        &self,
        plugin_id: &PluginId,
    ) -> Result<AgentPluginAttachment, ConnectionError> {
        let connection = self
            .lifecycle
            .ensure_running(plugin_id, AGENT_ATTACH_WAIT)
            .await?;
        let notifications = self.notifications.tap(plugin_id, connection.key());
        Ok(AgentPluginAttachment {
            connection,
            notifications,
        })
    }

    /// Replaces one Agent plugin's declarations and persists the merged consumer snapshot.
    ///
    /// Registration is process-scoped, while Effect surfaces are Workspace-scoped. Keeping the
    /// latest declaration per canonical Plugin ID lets independent Agent generations converge on
    /// one complete snapshot without one plugin accidentally retiring a sibling's surface.
    pub(crate) fn replace_agent_effect_surfaces(
        &self,
        plugin_id: PluginId,
        surfaces: Vec<FilesystemSkillSurface>,
    ) -> Result<(), BackendError> {
        let descriptors = {
            let mut registered = self
                .agent_effect_surfaces
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if surfaces.is_empty() {
                registered.remove(&plugin_id);
            } else {
                registered.insert(plugin_id, surfaces);
            }
            registered.values().flatten().cloned().collect::<Vec<_>>()
        };
        let timestamp = self.clock.now_timestamp_millis();
        let workspaces = self
            .workspace_repository
            .list_all_workspaces()
            .map_err(|error| BackendError::internal("failed to list Effect Workspaces", error))?;
        for workspace in workspaces {
            let WorkspaceLocation::LocalFilesystem { path } = &workspace.location else {
                // The first adapter is deliberately filesystem-only. Remote Workspaces need a
                // provider-owned adapter instead of treating an opaque locator as a host path.
                continue;
            };
            let merged = SurfaceDescriptorSet::merge(&workspace.id, descriptors.clone())
                .map_err(|error| BackendError::internal("invalid Agent Effect surface", error))?;
            self.effect_repository
                .replace_surfaces(&workspace.id, Path::new(path), &merged, timestamp)
                .map_err(|error| {
                    BackendError::internal("failed to persist Agent Effect surfaces", error)
                })?;
        }
        // Waking after the commit, never before it: the request the worker will read is already
        // durable, so a wake lost to a crash costs a scan interval rather than a reconcile.
        if let Some(reconcile) = self.effect_reconcile.get() {
            reconcile.notify();
        }
        Ok(())
    }

    /// Returns the merged Effect surface declarations of every currently registered Agent plugin.
    ///
    /// This snapshot is the single source convergence reads. It is process-local on purpose: a
    /// plugin that is not running declares nothing, and a Workspace therefore owes it no surface
    /// until its next start republishes the declaration.
    pub(crate) fn agent_effect_surface_declarations(&self) -> Vec<FilesystemSkillSurface> {
        self.agent_effect_surfaces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    /// Stops one plugin process while leaving the installed plugin available.
    pub(crate) async fn stop(
        &self,
        request: StopPluginRequest,
    ) -> Result<StopPluginResponse, PluginLifecycleError> {
        self.lifecycle.stop_plugin(request).await
    }

    /// Stops and removes one plugin package plus its process-local state.
    pub(crate) async fn uninstall(
        &self,
        request: UninstallPluginRequest,
    ) -> Result<UninstallPluginResponse, BackendError> {
        let plugin_id = request.plugin_id.clone();
        let response = self.lifecycle.uninstall_plugin(request).await?;
        let plugin_id = PluginId::parse(&plugin_id)
            .map_err(|error| BackendError::internal("uninstalled plugin id is invalid", error))?;
        self.skill_repository
            .remove_plugin_skills(&plugin_id, self.clock.now_timestamp_millis())
            .map_err(|error| BackendError::internal("failed to remove plugin Skills", error))?;
        self.replace_agent_effect_surfaces(plugin_id, Vec::new())?;
        Ok(response)
    }
    /// Installs a marketplace plugin by resolving its release manifest from the synced sources and
    /// downloading, verifying, and extracting its package through the network-backed installer.
    ///
    /// The source registries are read only for the release `url`/`sha256` (the cached index
    /// carries display fields only), so this returns NotFound when the identifier is not in any
    /// checkout.
    pub(crate) async fn install(
        &self,
        request: InstallPluginRequest,
    ) -> Result<InstallPluginResponse, BackendError> {
        let (manifest, use_proxy) = self.resolve_marketplace_release(&request.plugin_id)?;
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
        let download_proxy = self.download_proxy_for(use_proxy)?;
        Installer::new(ReqwestDownloader::new(download_proxy))
            .install(
                &manifest,
                DownloadSource::Url(release_url.clone()),
                &self.data_directory,
            )
            .await
            .map_err(|error| BackendError::internal("failed to install plugin", error))?;
        self.finalize_new_install(&request.plugin_id).await?;
        ora_info!(plugin_id = %request.plugin_id, "installed marketplace plugin");
        Ok(InstallPluginResponse {
            plugin_id: request.plugin_id,
        })
    }

    /// Updates one installed marketplace plugin to the version its source publishes.
    ///
    /// The source resolution and proxy policy are identical to an install: the winning
    /// marketplace source decides whether the download goes through the configured proxy. The
    /// running process is stopped before its package is replaced, and the installed snapshot is
    /// rescanned afterwards so the new version becomes effective without a restart.
    pub(crate) async fn update(
        &self,
        request: UpdatePluginRequest,
    ) -> Result<UpdatePluginResponse, BackendError> {
        let (manifest, use_proxy) = self.resolve_marketplace_release(&request.plugin_id)?;
        let release_url = manifest
            .url()
            .ok_or_else(|| {
                BackendError::internal(
                    "marketplace plugin manifest is missing its release url",
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "missing release url"),
                )
            })?
            .as_url();
        ora_info!(plugin_id = %request.plugin_id, url = %release_url, "updating marketplace plugin");
        // The package directory is replaced while the plugin may be running, so the process is
        // stopped first; stopping a webview/skill/MCP package is a no-op.
        self.lifecycle
            .stop_plugin(StopPluginRequest {
                plugin_id: request.plugin_id.clone(),
            })
            .await
            .map_err(BackendError::from)?;
        let download_proxy = self.download_proxy_for(use_proxy)?;
        Installer::new(ReqwestDownloader::new(download_proxy))
            .update(
                &manifest,
                DownloadSource::Url(release_url.clone()),
                &self.data_directory,
            )
            .await
            .map_err(|error| BackendError::internal("failed to update plugin", error))?;
        self.finalize_new_install(&request.plugin_id).await?;
        ora_info!(plugin_id = %request.plugin_id, "updated marketplace plugin");
        Ok(UpdatePluginResponse {
            plugin_id: request.plugin_id,
        })
    }

    /// Resolves the release manifest for one marketplace identifier across the configured sources.
    ///
    /// Sources are consulted in precedence order, and the returned flag is the winning source's
    /// proxy policy so installs and updates honor the same per-source setting as the git sync.
    fn resolve_marketplace_release(
        &self,
        plugin_id: &str,
    ) -> Result<(PluginManifest, bool), BackendError> {
        let registry_sources = self.prepared_registry_sources()?;
        // A malformed identifier can never name a registry entry, so it is reported the same way
        // as an unknown one instead of leaking the id grammar as a separate error class.
        let plugin_id = PluginId::parse(plugin_id).map_err(|_| {
            BackendError::new(
                ErrorClassification::NotFound,
                PublicError::PluginNotFound(EmptyErrorParams {}),
                "marketplace plugin id is not a valid `<namespace>/<name>`",
            )
        })?;
        for (source, use_proxy) in &registry_sources {
            let registry_dir = source.checkout_dir().join("registry");
            if let Some(manifest) = RegistryIndex::resolve_manifest(&registry_dir, &plugin_id)
                .map_err(|error| {
                    BackendError::internal("failed to resolve plugin release manifest", error)
                })?
            {
                return Ok((manifest, *use_proxy));
            }
        }
        Err(BackendError::new(
            ErrorClassification::NotFound,
            PublicError::PluginNotFound(EmptyErrorParams {}),
            "marketplace plugin was not found in the registry",
        ))
    }

    /// Returns the downloader proxy configuration for one marketplace source's proxy policy.
    fn download_proxy_for(&self, use_proxy: bool) -> Result<ProxyConfig, BackendError> {
        if !use_proxy {
            return Ok(ProxyConfig::default());
        }
        let proxy_settings = self.user_config.network_proxy_settings()?;
        proxy::download_proxy(proxy_settings.as_ref())?.ok_or_else(|| {
            BackendError::invalid_proxy_settings(
                "a marketplace source uses the proxy but no proxy is configured",
            )
        })
    }

    /// Imports a local `.orax` release archive: verifies and extracts it, refreshes the installed
    /// snapshot so the plugin is immediately usable without a restart.
    pub(crate) async fn import(
        &self,
        request: ImportPluginRequest,
    ) -> Result<ImportPluginResponse, BackendError> {
        let archive_path = PathBuf::from(&request.path);
        ora_info!(path = %request.path, "importing plugin release from local archive");
        // Extracting and verifying the archive is CPU/IO bound, so it runs on the blocking
        // pool instead of a tokio worker thread; the downloader is cloned only for the task
        // and is needed because `install_local` is an `Installer` method.
        let installer = self.installer.clone();
        let data_directory = self.data_directory.clone();
        let package = tokio::task::spawn_blocking(move || {
            installer.install_local(&archive_path, &data_directory)
        })
        .await
        .map_err(|error| BackendError::internal("failed to join plugin import task", error))?
        .map_err(|error| BackendError::internal("failed to import plugin archive", error))?;
        self.finalize_new_install(&package.id).await?;
        ora_info!(plugin_id = %package.id, "imported plugin release from local archive");
        Ok(ImportPluginResponse {
            plugin_id: package.id,
        })
    }

    /// Refreshes the installed-plugin snapshot after a new package lands.
    ///
    /// The installed snapshot is built once at startup, so a fresh install must re-scan for the
    /// new package to appear in the installed list without restarting the backend.
    async fn finalize_new_install(&self, plugin_id: &str) -> Result<(), BackendError> {
        self.sync_plugin_skills(plugin_id)?;
        if let Err(error) = self.lifecycle.scan_plugins(ScanPluginsRequest {}).await {
            ora_warn!(plugin_id = %plugin_id, %error, "installed the package but failed to refresh the installed-plugin snapshot");
        }
        Ok(())
    }

    /// Projects validated static Skill metadata into the shared catalog and Effect source tables.
    fn sync_plugin_skills(&self, plugin_id: &str) -> Result<(), BackendError> {
        let manager = PluginManager::discover(&self.data_directory);
        let plugin = manager
            .installed_plugins()
            .iter()
            .find(|plugin| plugin.id.canonical() == plugin_id)
            .ok_or_else(|| {
                BackendError::internal(
                    "installed plugin was not discoverable for Skill synchronization",
                    std::io::Error::new(std::io::ErrorKind::NotFound, plugin_id.to_string()),
                )
            })?;
        self.persist_discovered_plugin_skills(plugin)
    }

    /// Persists one already-discovered Skill plugin without exposing package layout to callers.
    fn persist_discovered_plugin_skills(
        &self,
        plugin: &ora_plugin_manager::InstalledPlugin,
    ) -> Result<(), BackendError> {
        let PluginContribution::Skill(descriptor) = &plugin.contributes else {
            self.skill_repository
                .remove_plugin_skills(&plugin.id, self.clock.now_timestamp_millis())
                .map_err(|error| {
                    BackendError::internal("failed to clear stale plugin Skills", error)
                })?;
            return Ok(());
        };
        let mut projections = Vec::with_capacity(descriptor.skills.len());
        for skill in &descriptor.skills {
            let manifest_path = skill.package_root.join("SKILL.md");
            let manifest = std::fs::read(&manifest_path).map_err(|error| {
                BackendError::internal(
                    "failed to read validated plugin Skill manifest",
                    std::io::Error::new(
                        error.kind(),
                        format!("{}: {error}", manifest_path.display()),
                    ),
                )
            })?;
            projections.push(PluginSkillProjection {
                name: skill.name.clone(),
                description: skill.description.clone(),
                package_root: skill.package_root.clone(),
                skill_md_digest: Digest::sha256(&manifest).to_string(),
            });
        }
        self.skill_repository
            .replace_plugin_skills(
                &plugin.id,
                &plugin.version.to_string(),
                &projections,
                self.clock.now_timestamp_millis(),
            )
            .map_err(|error| BackendError::internal("failed to persist plugin Skills", error))
    }
}

/** Maps marketplace source configuration failures onto their public classifications. */
fn map_marketplace_source_error(error: MarketplaceSourceStoreError) -> BackendError {
    match error {
        MarketplaceSourceStoreError::Validation(error) => BackendError::new(
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            format!("invalid plugin marketplace source: {error}"),
        ),
        MarketplaceSourceStoreError::Duplicate(url) => BackendError::new(
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            format!("plugin marketplace source already exists: {url}"),
        ),
        MarketplaceSourceStoreError::NotFound(url) => BackendError::new(
            ErrorClassification::NotFound,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            format!("plugin marketplace source was not found: {url}"),
        ),
        error => BackendError::internal(
            "failed to persist configured plugin marketplace sources",
            error,
        ),
    }
}

/// Converts one registry entry into the frontend-facing marketplace summary.
fn available_plugin(entry: &RegistryEntry) -> ora_contracts::AvailablePlugin {
    ora_contracts::AvailablePlugin {
        id: entry.id().canonical(),
        name: entry.name().to_owned(),
        title: entry.title().to_owned(),
        kind: entry.kind().to_owned(),
        namespace: entry.namespace().to_owned(),
        version: entry.version().to_string(),
        description: entry.description().to_owned(),
        logo: entry.logo().map(str::to_owned),
    }
}

#[cfg(test)]
mod tests {
    use super::BroadcastNotificationSink;
    use ora_domain::PluginId;
    use ora_plugin_lifecycle::{InboundNotification, PluginGenerationKey, PluginNotificationSink};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies a tap sees only its own plugin generation, in order, and that dropping the
    /// receiver releases the tap instead of failing later publications.
    #[tokio::test]
    async fn taps_receive_only_their_generation_and_release_on_drop() {
        let sink = BroadcastNotificationSink::new();
        let plugin_id = PluginId::new("official", "ora-space.agent").expect("plugin id");
        let notification = |generation: u64, method: &str| InboundNotification {
            plugin_id: plugin_id.clone(),
            generation: PluginGenerationKey(generation),
            method: method.to_owned(),
            params: json!({}),
        };
        let mut tap = sink.tap(&plugin_id, PluginGenerationKey(2));
        sink.on_notification(notification(1, "agent/acp"));
        sink.on_notification(notification(2, "agent/acp"));
        sink.on_notification(notification(2, "agent/modelsChanged"));

        let received = (
            tap.recv().await.expect("first"),
            tap.recv().await.expect("second"),
        );
        drop(tap);
        sink.on_notification(notification(2, "agent/acp"));

        assert_eq!(
            (received, sink.taps.lock().expect("lock taps").len(),),
            (
                (
                    notification(2, "agent/acp"),
                    notification(2, "agent/modelsChanged"),
                ),
                0,
            )
        );
    }

    /// Verifies a subscriber attached after construction receives notifications in order and a
    /// notification without any subscriber is dropped without panicking.
    #[tokio::test]
    async fn broadcasts_to_late_subscribers_and_tolerates_none() {
        let sink = BroadcastNotificationSink::new();
        let notification = |method: &str| InboundNotification {
            plugin_id: PluginId::new("official", "acme.panel").expect("plugin id"),
            generation: PluginGenerationKey(1),
            method: method.to_owned(),
            params: json!({ "n": 1 }),
        };
        sink.on_notification(notification("dropped"));

        let mut receiver = sink.subscribe();
        sink.on_notification(notification("ora/ui/push"));
        sink.on_notification(notification("ora/ui/other"));

        assert_eq!(
            (
                receiver.recv().await.expect("first"),
                receiver.recv().await.expect("second"),
            ),
            (notification("ora/ui/push"), notification("ora/ui/other"))
        );
    }
}
