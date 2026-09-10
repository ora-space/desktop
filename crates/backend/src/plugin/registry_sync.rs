//! The cached marketplace registry index: reading it, rebuilding it, and admitting rebuilds.
//!
//! A rebuild drives the Git CLI over shared source checkouts and then replaces one cache file, so
//! only one may run at a time. Because every rebuild produces the same index for every caller,
//! the excess callers are turned away rather than queued: the rebuild already in flight is
//! producing the very result they asked for.

use super::PluginApi;
use super::listing::available_plugin;
use crate::error::BackendError;
use gitlancer::{CliGitRunner, Git};
use ora_contracts::{
    ListAvailablePluginsRequest, ListAvailablePluginsResponse, SyncAvailablePluginsRequest,
    SyncAvailablePluginsResponse,
};
use ora_logging::{ora_info, ora_warn};
use ora_plugin_registry::{RegistryIndex, RegistrySync};
use ora_utils::url::canonical_repository_url;
use std::collections::HashSet;
use std::sync::{MutexGuard, TryLockError};

impl PluginApi {
    /// Returns the cached marketplace registry index, excluding listings from disabled sources.
    pub(crate) fn list_available_plugins(
        &self,
        _request: ListAvailablePluginsRequest,
    ) -> Result<ListAvailablePluginsResponse, BackendError> {
        let mut response = match RegistryIndex::load(&self.registry_index_path) {
            Ok(index) => ListAvailablePluginsResponse {
                updated_at: index.updated_at(),
                plugins: index.plugins().iter().map(available_plugin).collect(),
            },
            // A cache this host cannot read is the same situation as one that was never written:
            // the endpoint never reaches the network, so the only remedy either way is the user
            // syncing once. Reporting an error instead would turn an index schema change into a
            // marketplace that appears broken.
            Err(error) if RegistryIndex::is_unusable_cache(&error) => {
                ora_warn!(%error, "rebuilding an unusable plugin registry index cache");
                ListAvailablePluginsResponse {
                    updated_at: 0,
                    plugins: Vec::new(),
                }
            }
            Err(error) => {
                return Err(BackendError::internal(
                    "failed to load plugin registry index",
                    error,
                ));
            }
        };
        let enabled_urls: HashSet<String> = self
            .enabled_marketplace_sources()?
            .into_iter()
            .map(|source| canonical_repository_url(&source.source().url))
            .collect();
        response
            .plugins
            .retain(|plugin| enabled_urls.contains(&canonical_repository_url(&plugin.source_url)));
        Ok(response)
    }

    /// Admits one marketplace index rebuild, or reports that another one is already running.
    ///
    /// The returned guard *is* the rebuild slot: it is released when dropped, whether or not a
    /// rebuild actually ran.
    pub(crate) fn try_begin_rebuild(&self) -> Option<MutexGuard<'_, ()>> {
        match self.rebuilding.try_lock() {
            Ok(slot) => Some(slot),
            // The slot guards no value, so a rebuild that panicked left nothing to corrupt.
            // Honouring the poison would refuse every later rebuild for the rest of the process,
            // which is the only lasting damage available here.
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// Pulls every marketplace source, merges their registry indexes, and atomically replaces the
    /// cache.
    ///
    /// A request that arrives while another rebuild holds the slot is answered from the cached
    /// index rather than queued, so one refresh serves them both.
    pub(crate) fn sync_available_plugins(
        &self,
        _request: SyncAvailablePluginsRequest,
    ) -> Result<SyncAvailablePluginsResponse, BackendError> {
        let Some(_slot) = self.try_begin_rebuild() else {
            ora_info!("marketplace rebuild already in flight; answering from the cached index");
            let cached = self.list_available_plugins(ListAvailablePluginsRequest {})?;
            return Ok(SyncAvailablePluginsResponse {
                updated_at: cached.updated_at,
                plugins: cached.plugins,
            });
        };
        self.rebuild_registry_index()
    }

    /// Syncs every configured source checkout and atomically replaces the cached registry index.
    ///
    /// Callers must already hold the rebuild slot handed out by [`Self::try_begin_rebuild`].
    pub(crate) fn rebuild_registry_index(
        &self,
    ) -> Result<SyncAvailablePluginsResponse, BackendError> {
        let git = Git::new(CliGitRunner);
        let registry_sources = self.prepared_registry_sources()?;
        for (source, _, _) in &registry_sources {
            RegistrySync::sync(&git, source)
                .map_err(|error| BackendError::internal("failed to sync plugin registry", error))?;
        }
        let synced: Vec<&ora_plugin_registry::RegistrySource> = registry_sources
            .iter()
            .map(|(source, _use_proxy, _s3_config)| source)
            .collect();
        let build =
            RegistryIndex::build_all(&synced, ora_logging::clock::now_local().unix_timestamp());
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
}
