//! The cached marketplace registry index: reading it, rebuilding it, and admitting rebuilds.
//!
//! A rebuild drives the Git CLI over shared source checkouts and then replaces one cache file, so
//! only one may run at a time. Because every rebuild produces the same index for every caller,
//! the excess callers are turned away rather than queued: the rebuild already in flight is
//! producing the very result they asked for.
//!
//! Sources are refreshed one by one and a source that fails does not fail the rebuild: its
//! previous listings are carried over and the failure is reported, so one unreachable repository
//! cannot freeze every other source's listings or make a transient outage look like a withdrawal.

use super::listing::available_plugin;
use super::{PluginApi, proxy};
use crate::error::BackendError;
use crate::marketplace_sources::map_marketplace_source_error;
use gitlancer::{CliGitRunner, Git};
use ora_application::Clock;
use ora_contracts::{
    ListAvailablePluginsRequest, ListAvailablePluginsResponse, MarketplaceSourceSyncFailure,
    SyncAvailablePluginsRequest, SyncAvailablePluginsResponse,
};
use ora_logging::{ora_info, ora_warn};
use ora_plugin_registry::{
    RegistryBuild, RegistryIndex, RegistrySource, RegistrySourceFailure, RegistrySync,
};
use ora_utils::url::canonical_repository_url;
use std::collections::HashSet;
use std::sync::{MutexGuard, TryLockError};

#[cfg(test)]
mod tests;

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
                failed_sources: index.source_failures().iter().map(sync_failure).collect(),
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
                    failed_sources: Vec::new(),
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
        // A source disabled or removed after it failed stops being reported: the warning explains
        // listings that are on screen, and a source the user turned off no longer has any.
        response
            .failed_sources
            .retain(|failure| enabled_urls.contains(&canonical_repository_url(&failure.url)));
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
                failed_sources: cached.failed_sources,
            });
        };
        self.rebuild_registry_index()
    }

    /// Syncs every configured source checkout and atomically replaces the cached registry index.
    ///
    /// Callers must already hold the rebuild slot handed out by [`Self::try_begin_rebuild`].
    ///
    /// Everything that belongs to one source's refresh — its proxy requirement and its Git work —
    /// fails only that source. Reading Ora's own state (the configured sources
    /// and their namespace bindings) still fails the rebuild, because without it no source can be
    /// refreshed or attributed correctly. The S3 retrieval settings are not read at all: they only
    /// matter when a release is downloaded, so a source whose S3 settings cannot be read still
    /// refreshes its listings.
    ///
    /// The cache is written even when no source could be refreshed: it is the only place the
    /// failure survives the call, and the marketplace page has to be able to explain why the
    /// listings it shows did not move.
    pub(crate) fn rebuild_registry_index(
        &self,
    ) -> Result<SyncAvailablePluginsResponse, BackendError> {
        let git = Git::new(CliGitRunner);
        // This entire Git/cache rebuild runs synchronously on the host's blocking executor.
        let mut host_proxy = None;
        let configured = self.enabled_marketplace_sources()?;
        let now_ms = self.clock.now_timestamp_millis();
        let previous = self.load_previous_registry_index();
        let mut sources = Vec::with_capacity(configured.len());
        let mut failures = Vec::new();
        for configured in &configured {
            let source = self
                .marketplace_sources
                .registry_source(configured.source(), now_ms)
                .map_err(map_marketplace_source_error)?;
            // The full error goes to the log; the recorded failure keeps only what the user can
            // act on, because it is shown on the marketplace page and persisted in the cache.
            let direct = Ok(ora_utils::http::HostProxy::direct());
            let policy = if configured.source().use_proxy {
                host_proxy.get_or_insert_with(|| {
                    self.sync_settings
                        .network_proxy_preferences()
                        .map_err(|error| {
                            BackendError::internal(
                                "could not read marketplace proxy configuration",
                                error,
                            )
                        })
                        .and_then(|preferences| proxy::host_proxy(&preferences))
                })
            } else {
                &direct
            };
            let git_env = policy
                .as_ref()
                .map_err(ToString::to_string)
                .and_then(|policy| {
                    proxy::git_proxy_env(source.url(), policy).map_err(|error| error.to_string())
                });
            let failure = match git_env.map(|git_env| source.clone().with_git_env(git_env)) {
                Ok(proxied) => RegistrySync::sync(&git, &proxied).err().map(|error| {
                    let failure = RegistrySourceFailure::from_sync_error(source.canonical_url(), &error);
                    let failure = if proxy::git_proxy_auth_failure(failure.message()) {
                        RegistrySourceFailure::new(source.canonical_url(), ora_utils::http::HostProxyError::Authentication.to_string())
                    } else if proxy::git_proxy_redirect_failure(failure.message()) {
                        RegistrySourceFailure::new(
                            source.canonical_url(),
                            "Git refused a redirect. Use the final Git repository URL.",
                        )
                    } else {
                        failure
                    };
                    ora_warn!(
                        message = "marketplace source refresh failed; keeping its previous listings",
                        source = %source.canonical_url(),
                        error = %failure.message(),
                    );
                    failure
                }),
                Err(error) => {
                    ora_warn!(
                        message = "marketplace source proxy is unusable; keeping its previous listings",
                        source = %source.canonical_url(),
                        error = %error,
                    );
                    Some(RegistrySourceFailure::new(
                        source.canonical_url(),
                        error.to_string(),
                    ))
                }
            };
            failures.extend(failure);
            sources.push(source);
        }
        let synced: Vec<&RegistrySource> = sources.iter().collect();
        let build = compose_rebuild(
            &synced,
            &failures,
            previous.as_ref(),
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
            failed_sources: build
                .index()
                .source_failures()
                .iter()
                .map(sync_failure)
                .collect(),
        })
    }

    /// Loads the cached index when it can still be read, so a rebuild can carry over the listings
    /// of the sources it fails to refresh.
    fn load_previous_registry_index(&self) -> Option<RegistryIndex> {
        match RegistryIndex::load(&self.registry_index_path) {
            Ok(index) => Some(index),
            // "Not written yet" and "written by another schema" mean the same thing here: there
            // are no previous listings to carry over, and the rebuild that follows either fills
            // the cache or reports why it could not.
            Err(error) if RegistryIndex::is_unusable_cache(&error) => None,
            Err(error) => {
                ora_warn!(
                    %error,
                    "reading the previous plugin registry index failed; a failed source keeps no listings this round"
                );
                None
            }
        }
    }
}

/// Maps one recorded source failure into the contract shape the UI reports.
fn sync_failure(failure: &RegistrySourceFailure) -> MarketplaceSourceSyncFailure {
    MarketplaceSourceSyncFailure {
        url: failure.url().to_owned(),
        message: failure.message().to_owned(),
    }
}

/// Composes one rebuild from what every source did, keeping the listings of the sources that
/// failed and reporting a sync time that is actually true.
///
/// The sync time only moves when at least one source refreshed: a rebuild that reached none of
/// them carries exactly the listings the previous index already had, so reporting "synced now"
/// would claim a refresh that never happened. With no previous index there is no sync time to
/// report either, and `0` ("never synced") is the truthful answer. A rebuild with no enabled
/// source at all failed nothing: the empty catalog it writes is accurate as of now.
fn compose_rebuild(
    sources: &[&RegistrySource],
    failures: &[RegistrySourceFailure],
    previous: Option<&RegistryIndex>,
    refreshed_at: i64,
) -> RegistryBuild {
    let every_source_failed = !sources.is_empty()
        && sources.iter().all(|source| {
            failures
                .iter()
                .any(|failure| failure.url() == source.canonical_url())
        });
    let updated_at = if every_source_failed {
        previous.map_or(0, RegistryIndex::updated_at)
    } else {
        refreshed_at
    };
    RegistryIndex::build_with_failures(sources, failures, previous, updated_at)
}
