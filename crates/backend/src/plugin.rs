use crate::app_event::AppEventPublisher;
use crate::clock::SystemClock;
use crate::error::{BackendError, ErrorClassification};
use gitlancer::{BranchName, CliGitRunner, Git};
use ora_application::Clock;
use ora_contracts::{
    ActivatePluginRequest, ActivatePluginResponse, DisablePluginRequest, DisablePluginResponse,
    EmptyErrorParams, EnablePluginRequest, EnablePluginResponse, ImportPluginRequest,
    ImportPluginResponse, InstallPluginRequest, InstallPluginResponse, ListAvailablePluginsRequest,
    ListAvailablePluginsResponse, ListInstalledPluginsRequest, ListInstalledPluginsResponse,
    PublicError, ScanPluginsRequest, ScanPluginsResponse, StopPluginRequest, StopPluginResponse,
    SyncAvailablePluginsRequest, SyncAvailablePluginsResponse, UninstallPluginRequest,
    UninstallPluginResponse,
};
use ora_db::{
    PluginSkillProjection, RepositoryPool, SqlitePluginStateRepository, SqliteSkillRepository,
};
use ora_domain::PluginId;
use ora_effect::Digest;
use ora_logging::{ora_debug, ora_info, ora_warn};
use ora_plugin_lifecycle::{
    ConnectionError, DenoPluginRuntime, DenoPluginRuntimeLauncher, InboundNotification,
    PluginGenerationKey, PluginGenerationLease, PluginLifecycle, PluginLifecycleConfig,
    PluginLifecycleError, PluginNotificationSink, PluginRuntimeTimeouts,
};
use ora_plugin_manager::{Installer, PluginContribution, PluginManager};
use ora_plugin_registry::{
    RegistryEntry, RegistryError, RegistryIndex, RegistrySource, RegistrySync,
};
use ora_utils::http::{DownloadSource, ProxyConfig, ReqwestDownloader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// The default marketplace sources this backend syncs, each tracked on its own branch.
///
/// Order matters only for overlapping plugin ids: when two sources publish the same plugin, the
/// earlier source wins both the merged listing and the install-time lookup.
const MARKETPLACE_SOURCES: &[(&str, &str)] =
    &[("https://github.com/ora-space/marketplace", "main")];

/// The concrete lifecycle composition the backend runs.
pub(crate) type BackendPluginLifecycle = PluginLifecycle<
    SqlitePluginStateRepository,
    SystemClock,
    DenoPluginRuntimeLauncher,
    AppEventPublisher,
    BroadcastNotificationSink,
>;

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
    lifecycle: BackendPluginLifecycle,
    registry_sources: Vec<RegistrySource>,
    registry_index_path: PathBuf,
    data_directory: PathBuf,
    installer: Installer<ReqwestDownloader>,
    notifications: BroadcastNotificationSink,
    skill_repository: SqliteSkillRepository,
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
    ) -> Result<Self, PluginLifecycleError> {
        let plugins_directory = data_directory.join("plugins");
        let sources_root = plugins_directory.join("sources");
        let registry_sources = MARKETPLACE_SOURCES
            .iter()
            .map(|(url, branch)| {
                RegistrySource::from_git(*url, BranchName::new(*branch), &sources_root)
            })
            .collect();
        let registry_index_path = plugins_directory.join("cache").join("registry_index.json");
        let installer = Installer::new(ReqwestDownloader::new(ProxyConfig::default()));
        let notifications = BroadcastNotificationSink::new();
        let lifecycle = PluginLifecycle::open(
            PluginLifecycleConfig {
                data_directory: data_directory.clone(),
                deno_path,
            },
            SqlitePluginStateRepository::new(pool.clone()),
            clock,
            DenoPluginRuntimeLauncher::new(PluginRuntimeTimeouts::default()),
            publisher,
            notifications.clone(),
        )?;

        Ok(Self {
            lifecycle,
            registry_sources,
            registry_index_path,
            data_directory,
            installer,
            notifications,
            skill_repository: SqliteSkillRepository::new(pool),
            clock,
        })
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

    /// Pulls every marketplace source, merges their registry indexes, and atomically replaces the
    /// cache.
    pub(crate) fn sync_available_plugins(
        &self,
        _request: SyncAvailablePluginsRequest,
    ) -> Result<SyncAvailablePluginsResponse, RegistryError> {
        let git = Git::new(CliGitRunner);
        let mut registry_dirs: Vec<PathBuf> = Vec::with_capacity(self.registry_sources.len());
        for source in &self.registry_sources {
            let checkout_directory = RegistrySync::sync(&git, source)?;
            registry_dirs.push(checkout_directory.join("registry"));
        }
        let registry_dir_refs: Vec<&Path> = registry_dirs.iter().map(PathBuf::as_path).collect();
        let build = RegistryIndex::build_all(
            &registry_dir_refs,
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

    /// Rescans packages and reconciles durable and runtime state.
    pub(crate) async fn scan(
        &self,
        request: ScanPluginsRequest,
    ) -> Result<ScanPluginsResponse, PluginLifecycleError> {
        self.lifecycle.scan_plugins(request).await
    }

    /// Persists plugin eligibility and starts the runtime it implies.
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

    /// Returns a connection to a running plugin plus a lossless stream of its notifications,
    /// starting the plugin when it is enabled but stopped.
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
    ) -> Result<UninstallPluginResponse, BackendError> {
        let plugin_id = request.plugin_id.clone();
        let response = self.lifecycle.uninstall_plugin(request).await?;
        let plugin_id = PluginId::parse(&plugin_id)
            .map_err(|error| BackendError::internal("uninstalled plugin id is invalid", error))?;
        self.skill_repository
            .remove_plugin_skills(&plugin_id, self.clock.now_timestamp_millis())
            .map_err(|error| BackendError::internal("failed to remove plugin Skills", error))?;
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
        let registry_dirs: Vec<PathBuf> = self
            .registry_sources
            .iter()
            .map(|source| source.checkout_dir().join("registry"))
            .collect();
        let registry_dir_refs: Vec<&Path> = registry_dirs.iter().map(PathBuf::as_path).collect();
        // A malformed identifier can never name a registry entry, so it is reported the same way
        // as an unknown one instead of leaking the id grammar as a separate error class.
        let plugin_id = PluginId::parse(&request.plugin_id).map_err(|_| {
            BackendError::new(
                ErrorClassification::NotFound,
                PublicError::PluginNotFound(EmptyErrorParams {}),
                "marketplace plugin id is not a valid `<namespace>/<name>`",
            )
        })?;
        let manifest = RegistryIndex::resolve_manifest_all(&registry_dir_refs, &plugin_id)
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
        self.finalize_new_install(&request.plugin_id).await?;
        ora_info!(plugin_id = %request.plugin_id, "installed marketplace plugin");
        Ok(InstallPluginResponse {
            plugin_id: request.plugin_id,
        })
    }

    /// Imports a local `.orax` release archive: verifies and extracts it, refreshes the installed
    /// snapshot, and enables the plugin so it is immediately usable without a restart.
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

    /// Refreshes the installed-plugin snapshot after a new package lands, then enables it by
    /// default so the frontend surface reports it as immediately usable.
    ///
    /// The installed snapshot is built once at startup, so a fresh install must re-scan for the
    /// new package to appear in the installed list without restarting the backend. Enabling is a
    /// best-effort follow-up: a package that fails to launch still reports its failure through the
    /// lifecycle runtime instead of failing the install.
    async fn finalize_new_install(&self, plugin_id: &str) -> Result<(), BackendError> {
        self.sync_plugin_skills(plugin_id)?;
        if let Err(error) = self.lifecycle.scan_plugins(ScanPluginsRequest {}).await {
            ora_warn!(plugin_id = %plugin_id, %error, "installed the package but failed to refresh the installed-plugin snapshot");
        }
        if let Err(error) = self
            .lifecycle
            .enable_plugin(EnablePluginRequest {
                plugin_id: plugin_id.to_string(),
            })
            .await
        {
            ora_warn!(plugin_id = %plugin_id, %error, "installed plugin could not be enabled by default");
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
