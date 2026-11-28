mod artifact_retrieval;
mod error;

use artifact_retrieval::StoredArtifactRetrieval;
pub(crate) use error::{MarketplaceSourceStoreError, map_marketplace_source_error};
use ora_contracts::{
    MarketplaceArtifactRetrieval, MarketplaceSource, UpdateMarketplaceSourceRequest,
};
use ora_db::{
    PluginMarketplaceSourceRecord, SqlitePluginMarketplaceSourceRepository,
    SqlitePluginSourceNamespaceRepository,
};
use ora_domain::PluginNamespace;
use ora_plugin_registry::{RegistryError, RegistrySource};
use ora_utils::url::canonical_repository_url;
use std::path::{Path, PathBuf};

/// The seed source used the first time a backend opens before any user configuration exists.
const DEFAULT_MARKETPLACE_URL: &str = PluginNamespace::DEFAULT_MARKETPLACE_URL;
const DEFAULT_MARKETPLACE_BRANCH: &str = "main";

/// Owns SQLite-backed marketplace source configuration and binds each row to a namespace and a
/// checkout path.
pub(crate) struct MarketplaceSourceStore {
    repository: SqlitePluginMarketplaceSourceRepository,
    namespaces: SqlitePluginSourceNamespaceRepository,
    sources_root: PathBuf,
}

/// Keeps the secret-bearing retrieval state beside its redacted public source projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfiguredMarketplaceSource {
    source: MarketplaceSource,
    artifact_retrieval: StoredArtifactRetrieval,
}

impl ConfiguredMarketplaceSource {
    /// Returns the redacted source fields safe for contracts and ordinary diagnostics.
    pub(crate) fn source(&self) -> &MarketplaceSource {
        &self.source
    }

    /// Propagates invalid S3 configuration without falling back to unsigned retrieval.
    pub(crate) fn s3_config(
        &self,
    ) -> Result<Option<ora_utils::http::S3Config>, MarketplaceSourceStoreError> {
        self.artifact_retrieval.s3_config().map_err(Into::into)
    }
}

impl MarketplaceSourceStore {
    /// Seeds only a brand-new installation; an existing empty list is authoritative.
    pub(crate) fn open(
        repository: SqlitePluginMarketplaceSourceRepository,
        namespaces: SqlitePluginSourceNamespaceRepository,
        home_directory: &Path,
        now_ms: i64,
    ) -> Result<Self, MarketplaceSourceStoreError> {
        let sources_root = home_directory.join("plugins").join("sources");
        let store = Self {
            repository,
            namespaces,
            sources_root,
        };
        if store.repository.needs_initial_source()? {
            if store.repository.list_sources()?.is_empty() {
                let default_source = MarketplaceSource {
                    url: DEFAULT_MARKETPLACE_URL.to_owned(),
                    branch: DEFAULT_MARKETPLACE_BRANCH.to_owned(),
                    use_proxy: false,
                    enabled: true,
                    artifact_retrieval: MarketplaceArtifactRetrieval::DirectHttps,
                };
                store.insert(
                    default_source,
                    StoredArtifactRetrieval::DirectHttps,
                    /*position*/ 0,
                    now_ms,
                )?;
            }
            // If startup stops after insertion, retry sees the row and only consumes the marker.
            store.repository.finish_source_initialization()?;
        }
        Ok(store)
    }

    /// Returns the current source list in source-precedence order.
    pub(crate) fn list(&self) -> Result<Vec<MarketplaceSource>, MarketplaceSourceStoreError> {
        Ok(self
            .configured_sources()?
            .into_iter()
            .map(|configured| configured.source)
            .collect())
    }

    /// Returns validated source configuration including secret-bearing retrieval state.
    pub(crate) fn configured_sources(
        &self,
    ) -> Result<Vec<ConfiguredMarketplaceSource>, MarketplaceSourceStoreError> {
        self.repository
            .list_sources()?
            .into_iter()
            .map(configured_source)
            .collect()
    }

    /// Validates, persists, and returns one additional source appended to the current ordering.
    pub(crate) fn add(
        &self,
        source: MarketplaceSource,
        now_ms: i64,
    ) -> Result<Vec<MarketplaceSource>, MarketplaceSourceStoreError> {
        let current = self.repository.list_sources()?;
        // Duplicates are rejected on the canonical URL, not the configured spelling: two rows
        // pointing at one repository would share a namespace and a checkout, so they would be the
        // same source listed twice rather than two sources.
        let canonical = canonical_repository_url(&source.url);
        if current
            .iter()
            .any(|existing| canonical_repository_url(&existing.url) == canonical)
        {
            return Err(MarketplaceSourceStoreError::Duplicate(source.url));
        }
        checked_source(&source, PluginNamespace::official(), &self.sources_root)?;
        let position = current
            .iter()
            .map(|existing| existing.position)
            .max()
            .map_or(0, |highest| highest + 1);
        self.insert(
            source,
            StoredArtifactRetrieval::DirectHttps,
            position,
            now_ms,
        )?;
        self.list()
    }

    /// Removes one source by URL and returns the remaining sources.
    ///
    /// The namespace binding is intentionally left behind: plugins installed from this source
    /// still carry it in their install paths and durable rows, so re-adding the repository later
    /// must resolve to the same identity rather than mint a second one.
    pub(crate) fn delete(
        &self,
        url: &str,
    ) -> Result<Vec<MarketplaceSource>, MarketplaceSourceStoreError> {
        if !self.repository.delete_source(url)? {
            return Err(MarketplaceSourceStoreError::NotFound(url.to_owned()));
        }
        self.list()
    }

    /// Replaces the editable fields of one source and returns the authoritative list.
    ///
    /// Changing the Git URL to a different canonical repository binds a new namespace for that
    /// address the same way add does. Equivalent spellings of the current repository reuse the
    /// existing binding so installed plugins stay attached.
    pub(crate) fn update(
        &self,
        request: UpdateMarketplaceSourceRequest,
        now_ms: i64,
    ) -> Result<Vec<MarketplaceSource>, MarketplaceSourceStoreError> {
        let current = self.repository.list_sources()?;
        let Some(existing) = current.iter().find(|row| row.url == request.url).cloned() else {
            return Err(MarketplaceSourceStoreError::NotFound(request.url));
        };
        let new_canonical = canonical_repository_url(&request.new_url);
        if current.iter().any(|row| {
            row.url != request.url && canonical_repository_url(&row.url) == new_canonical
        }) {
            return Err(MarketplaceSourceStoreError::Duplicate(request.new_url));
        }
        let existing_retrieval = StoredArtifactRetrieval::parse(&existing.artifact_retrieval)?;
        let artifact_retrieval =
            StoredArtifactRetrieval::updated(&existing_retrieval, request.artifact_retrieval)?;
        let source = MarketplaceSource {
            url: request.new_url,
            branch: request.branch,
            use_proxy: request.use_proxy,
            enabled: request.enabled,
            artifact_retrieval: artifact_retrieval.public(),
        };
        let namespace = self.namespace_for(&source.url, now_ms)?;
        checked_source(&source, namespace, &self.sources_root)?;
        if !self.repository.update_source(
            &request.url,
            &PluginMarketplaceSourceRecord {
                url: source.url,
                branch: source.branch,
                use_proxy: source.use_proxy,
                enabled: source.enabled,
                artifact_retrieval: artifact_retrieval.to_json()?,
                position: existing.position,
            },
            now_ms,
        )? {
            return Err(MarketplaceSourceStoreError::NotFound(request.url));
        }
        self.list()
    }

    /// Binds one contract-shaped source to a validated [`RegistrySource`] carrying the namespace
    /// this source publishes under.
    pub(crate) fn registry_source(
        &self,
        source: &MarketplaceSource,
        now_ms: i64,
    ) -> Result<RegistrySource, MarketplaceSourceStoreError> {
        let namespace = self.namespace_for(&source.url, now_ms)?;
        checked_source(source, namespace, &self.sources_root)
            .map_err(MarketplaceSourceStoreError::Validation)
    }

    /// Resolves the namespace of one source URL, binding it on first use.
    ///
    /// The default marketplace short-circuits to the reserved `official` namespace: it is the
    /// source almost every plugin comes from, and `official/<name>` is far more legible in install
    /// paths, logs, and errors than a digest-suffixed slug would be. Every other source derives
    /// its namespace from its canonical URL, and the result is persisted the first time so that
    /// later changes to the normalization rules cannot move an installed plugin's identity: the
    /// derivation is the contract, and the stored row is this machine's record of honoring it.
    fn namespace_for(
        &self,
        url: &str,
        now_ms: i64,
    ) -> Result<PluginNamespace, MarketplaceSourceStoreError> {
        let canonical = canonical_repository_url(url);
        if canonical == canonical_repository_url(DEFAULT_MARKETPLACE_URL) {
            return Ok(PluginNamespace::official());
        }
        let derived = PluginNamespace::derive_from_canonical_url(&canonical);
        let bound = self.namespaces.bind(&canonical, derived.as_str(), now_ms)?;
        Ok(PluginNamespace::parse(&bound)?)
    }

    /// Inserts one already-validated source at the supplied precedence position.
    fn insert(
        &self,
        source: MarketplaceSource,
        artifact_retrieval: StoredArtifactRetrieval,
        position: i64,
        now_ms: i64,
    ) -> Result<(), MarketplaceSourceStoreError> {
        // Binding the namespace as the source lands keeps the identity a purely local decision:
        // adding a source never has to reach the network to learn what its plugins will be called.
        self.namespace_for(&source.url, now_ms)?;
        self.repository.insert_source(
            &PluginMarketplaceSourceRecord {
                url: source.url,
                branch: source.branch,
                use_proxy: source.use_proxy,
                enabled: source.enabled,
                artifact_retrieval: artifact_retrieval.to_json()?,
                position,
            },
            now_ms,
        )?;
        Ok(())
    }
}

/// Validates and binds one wire source to its namespace and derived checkout directory.
fn checked_source(
    source: &MarketplaceSource,
    namespace: PluginNamespace,
    sources_root: &Path,
) -> Result<RegistrySource, RegistryError> {
    RegistrySource::try_from_git(
        source.url.clone(),
        namespace,
        source.branch.clone(),
        sources_root,
    )
}

/// Projects one durable row back to the frontend-facing wire shape.
fn configured_source(
    source: PluginMarketplaceSourceRecord,
) -> Result<ConfiguredMarketplaceSource, MarketplaceSourceStoreError> {
    let artifact_retrieval = StoredArtifactRetrieval::parse(&source.artifact_retrieval)?;
    Ok(ConfiguredMarketplaceSource {
        source: MarketplaceSource {
            url: source.url,
            branch: source.branch,
            use_proxy: source.use_proxy,
            enabled: source.enabled,
            artifact_retrieval: artifact_retrieval.public(),
        },
        artifact_retrieval,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog,
    };
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn open_pool(temp: &TempDir) -> RepositoryPool {
        ora_logging::with_trace_logging(|| {
            let database_path = temp.path().join("ora.sqlite3");
            DatabaseBootstrapper::new(crate::test_clock::TestClock)
                .bootstrap_repository_pool(
                    &DatabaseLocation::path(&database_path),
                    &default_migration_catalog().expect("build migration catalog"),
                )
                .expect("open repository pool")
        })
    }

    fn store_with_pool(temp: &TempDir, pool: RepositoryPool) -> MarketplaceSourceStore {
        MarketplaceSourceStore::open(
            SqlitePluginMarketplaceSourceRepository::new(pool.clone()),
            SqlitePluginSourceNamespaceRepository::new(pool),
            temp.path(),
            1,
        )
        .expect("open store")
    }

    fn store_with_repository(temp: &TempDir) -> (RepositoryStoreGuard, MarketplaceSourceStore) {
        let pool = open_pool(temp);
        (RepositoryStoreGuard, store_with_pool(temp, pool))
    }

    fn third_party(url: &str) -> MarketplaceSource {
        MarketplaceSource {
            url: url.to_owned(),
            branch: "main".to_owned(),
            use_proxy: false,
            enabled: true,
            artifact_retrieval: MarketplaceArtifactRetrieval::DirectHttps,
        }
    }

    struct RepositoryStoreGuard;

    #[test]
    fn missing_sources_seed_the_default_source() {
        let temp = TempDir::new().expect("create temp directory");
        let (_, store) = store_with_repository(&temp);

        assert_eq!(
            store.list().expect("list sources"),
            vec![MarketplaceSource {
                url: DEFAULT_MARKETPLACE_URL.to_owned(),
                branch: DEFAULT_MARKETPLACE_BRANCH.to_owned(),
                use_proxy: false,
                enabled: true,
                artifact_retrieval: MarketplaceArtifactRetrieval::DirectHttps,
            }]
        );
    }

    /// Characterizes startup reseeding after the user deliberately removed every source.
    #[test]
    fn empty_user_source_list_is_preserved_on_reopen() {
        ora_logging::with_trace_logging(|| {
            let temp = TempDir::new().expect("create temp directory");
            let (_, store) = store_with_repository(&temp);
            assert_eq!(store.delete(DEFAULT_MARKETPLACE_URL).unwrap(), vec![]);
            drop(store);
            let (_, reopened) = store_with_repository(&temp);
            assert_eq!(reopened.list().unwrap(), vec![]);
        });
    }

    /// Reproduces the old release: deleting the last source wrote no user_config marker.
    #[test]
    fn legacy_empty_sources_survive_upgrade_without_a_deletion_marker() {
        ora_logging::with_trace_logging(|| {
            let temp = TempDir::new().unwrap();
            let latest = default_migration_catalog().unwrap();
            let versions = latest.target_versions()[..21].to_vec();
            let migrations = latest
                .target_versions()
                .iter()
                .map(|version| latest.migration(version).unwrap().clone())
                .collect();
            let old_catalog =
                ora_db::MigrationCatalog::with_target_versions(migrations, versions).unwrap();
            let location = DatabaseLocation::path(temp.path().join("ora.sqlite3"));
            let pool = DatabaseBootstrapper::system()
                .bootstrap_repository_pool(&location, &old_catalog)
                .unwrap();
            let repository = SqlitePluginMarketplaceSourceRepository::new(pool.clone());
            repository
                .insert_source(
                    &PluginMarketplaceSourceRecord {
                        url: DEFAULT_MARKETPLACE_URL.to_owned(),
                        branch: "main".to_owned(),
                        use_proxy: false,
                        enabled: true,
                        artifact_retrieval: r#"{"type":"direct_https"}"#.to_owned(),
                        position: 0,
                    },
                    1,
                )
                .unwrap();
            repository.delete_source(DEFAULT_MARKETPLACE_URL).unwrap();
            // Released databases had no bootstrap provenance table.
            rusqlite::Connection::open(temp.path().join("ora.sqlite3"))
                .unwrap()
                .execute_batch("DROP TABLE bootstrap_state")
                .unwrap();
            // No new-version store or seeding API has run before this upgrade boundary.
            drop(repository);
            drop(pool);
            for _ in 0..2 {
                let upgraded = DatabaseBootstrapper::system()
                    .bootstrap_repository_pool(&location, &latest)
                    .unwrap();
                let store = store_with_pool(&temp, upgraded);
                assert!(store.list().unwrap().is_empty());
            }
        });
    }

    #[test]
    fn first_install_with_distributor_sources_does_not_add_or_restore_public_source() {
        let temp = TempDir::new().unwrap();
        let pool = open_pool(&temp);
        let repository = SqlitePluginMarketplaceSourceRepository::new(pool.clone());
        repository
            .insert_source(
                &PluginMarketplaceSourceRecord {
                    url: "https://example.com/private".to_owned(),
                    branch: "private".to_owned(),
                    use_proxy: true,
                    enabled: false,
                    artifact_retrieval: r#"{"type":"direct_https"}"#.to_owned(),
                    position: 0,
                },
                1,
            )
            .unwrap();
        let store = store_with_pool(&temp, pool.clone());
        let sources = store.list().unwrap();
        assert_eq!(
            sources,
            vec![MarketplaceSource {
                url: "https://example.com/private".to_owned(),
                branch: "private".to_owned(),
                use_proxy: true,
                enabled: false,
                artifact_retrieval: MarketplaceArtifactRetrieval::DirectHttps,
            }]
        );
        store.delete("https://example.com/private").unwrap();
        drop(store);
        assert!(store_with_pool(&temp, pool).list().unwrap().is_empty());
    }

    /// Replacing distribution defaults cannot disable the Marketplace's initialization policy.
    #[test]
    fn distributor_sql_without_sources_still_seeds_the_public_default() {
        ora_logging::with_trace_logging(|| {
            let temp = TempDir::new().unwrap();
            let catalog = default_migration_catalog()
                .unwrap()
                .with_first_install_sql(&[
                    "INSERT INTO user_config (key, value) VALUES ('developer_mode', 'true');",
                ]);
            let pool = DatabaseBootstrapper::system()
                .bootstrap_repository_pool(
                    &DatabaseLocation::path(temp.path().join("ora.sqlite3")),
                    &catalog,
                )
                .unwrap();
            assert_eq!(
                store_with_pool(&temp, pool).list().unwrap(),
                vec![third_party(DEFAULT_MARKETPLACE_URL)]
            );
        });
    }

    /// Fresh provenance survives failures before/after schema commits and in distributor data.
    #[test]
    fn interrupted_first_install_resumes_default_source_initialization() {
        ora_logging::with_trace_logging(|| {
            let latest = default_migration_catalog().unwrap();
            let count = latest.target_versions().len();
            for failure_at in [0, 1, count] {
                let temp = TempDir::new().unwrap();
                let location = DatabaseLocation::path(temp.path().join("ora.sqlite3"));
                let mut migrations: Vec<_> = latest
                    .target_versions()
                    .iter()
                    .map(|version| latest.migration(version).unwrap().clone())
                    .collect();
                if failure_at < count {
                    migrations[failure_at] = ora_db::Migration::new(
                        latest.target_versions()[failure_at],
                        &["SELECT * FROM injected_first_install_failure;"],
                        &[],
                    );
                }
                let mut failing = ora_db::MigrationCatalog::new(migrations).unwrap();
                if failure_at == count {
                    failing = failing
                        .with_first_install_sql(&["SELECT * FROM injected_first_install_failure;"]);
                }
                assert!(
                    DatabaseBootstrapper::system()
                        .bootstrap_repository_pool(&location, &failing)
                        .is_err()
                );
                let partial = RepositoryPool::new(&location).unwrap();
                let committed = rusqlite::Connection::open(temp.path().join("ora.sqlite3"))
                    .unwrap()
                    .query_row("SELECT COUNT(*) FROM migrations", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .unwrap();
                assert_eq!(committed, failure_at as i64);
                assert!(
                    SqlitePluginMarketplaceSourceRepository::new(partial.clone())
                        .needs_initial_source()
                        .unwrap()
                );
                drop(partial);
                let recovered = DatabaseBootstrapper::system()
                    .bootstrap_repository_pool(&location, &latest)
                    .unwrap();
                let store = store_with_pool(&temp, recovered.clone());
                assert_eq!(
                    store.list().unwrap(),
                    vec![third_party(DEFAULT_MARKETPLACE_URL)]
                );
                store.delete(DEFAULT_MARKETPLACE_URL).unwrap();
                drop(store);
                assert_eq!(store_with_pool(&temp, recovered).list().unwrap(), vec![]);
            }
        });
    }

    #[test]
    fn add_update_and_delete_persist_sources() {
        let temp = TempDir::new().expect("create temp directory");
        let (_, store) = store_with_repository(&temp);
        let added = MarketplaceSource {
            url: "https://github.com/example/marketplace".to_owned(),
            branch: "main".to_owned(),
            use_proxy: true,
            enabled: true,
            artifact_retrieval: MarketplaceArtifactRetrieval::DirectHttps,
        };

        let sources = store.add(added.clone(), 2).expect("add source");
        assert_eq!(sources.len(), 2);
        assert!(sources.contains(&added));

        let updated = store
            .update(
                UpdateMarketplaceSourceRequest {
                    url: added.url.clone(),
                    new_url: added.url.clone(),
                    branch: "release".to_owned(),
                    use_proxy: false,
                    enabled: false,
                    artifact_retrieval:
                        ora_contracts::MarketplaceArtifactRetrievalUpdate::DirectHttps,
                },
                3,
            )
            .expect("update source");
        assert_eq!(
            updated[1],
            MarketplaceSource {
                url: added.url.clone(),
                branch: "release".to_owned(),
                use_proxy: false,
                enabled: false,
                artifact_retrieval: MarketplaceArtifactRetrieval::DirectHttps,
            }
        );

        let remaining = store.delete(&added.url).expect("delete source");
        assert_eq!(remaining.len(), 1);
    }

    /// S3 credentials are replaced as one pair and can be preserved without crossing the query contract.
    #[test]
    fn updates_and_preserves_s3_retrieval_credentials() {
        let temp = TempDir::new().expect("create temp directory");
        let pool = open_pool(&temp);
        let store = store_with_pool(&temp, pool.clone());
        let source = third_party("https://github.com/example/private-marketplace");
        store.add(source.clone(), 2).expect("add source");

        store
            .update(
                UpdateMarketplaceSourceRequest {
                    url: source.url.clone(),
                    new_url: source.url.clone(),
                    branch: source.branch.clone(),
                    use_proxy: false,
                    enabled: true,
                    artifact_retrieval:
                        ora_contracts::MarketplaceArtifactRetrievalUpdate::S3SigV4 {
                            endpoint: "https://s3.example.com:443".to_owned(),
                            bucket: "plugins".to_owned(),
                            region: "region-1".to_owned(),
                            credentials: ora_contracts::MarketplaceS3CredentialsUpdate::Replace {
                                access_key_id: "access".to_owned(),
                                secret_access_key: "secret".to_owned(),
                            },
                        },
                },
                3,
            )
            .expect("enable S3 retrieval");
        let public = store.list().expect("list redacted sources");
        assert_eq!(
            public[1].artifact_retrieval,
            MarketplaceArtifactRetrieval::S3SigV4 {
                endpoint: "https://s3.example.com".to_owned(),
                bucket: "plugins".to_owned(),
                region: "region-1".to_owned(),
            }
        );

        store
            .update(
                UpdateMarketplaceSourceRequest {
                    url: source.url.clone(),
                    new_url: source.url.clone(),
                    branch: source.branch,
                    use_proxy: false,
                    enabled: true,
                    artifact_retrieval:
                        ora_contracts::MarketplaceArtifactRetrievalUpdate::S3SigV4 {
                            endpoint: "https://objects.example.com".to_owned(),
                            bucket: "plugins-v2".to_owned(),
                            region: "region-2".to_owned(),
                            credentials: ora_contracts::MarketplaceS3CredentialsUpdate::Preserve,
                        },
                },
                4,
            )
            .expect("preserve S3 credentials");

        let records = SqlitePluginMarketplaceSourceRepository::new(pool.clone())
            .list_sources()
            .expect("load persisted source");
        let persisted = StoredArtifactRetrieval::parse(&records[1].artifact_retrieval)
            .expect("parse retrieval");
        assert_eq!(
            persisted,
            StoredArtifactRetrieval::S3SigV4 {
                endpoint: "https://objects.example.com".to_owned(),
                bucket: "plugins-v2".to_owned(),
                region: "region-2".to_owned(),
                access_key_id: "access".to_owned(),
                secret_access_key: "secret".to_owned(),
            }
        );

        store
            .update(
                UpdateMarketplaceSourceRequest {
                    url: source.url.clone(),
                    new_url: source.url,
                    branch: "main".to_owned(),
                    use_proxy: false,
                    enabled: true,
                    artifact_retrieval:
                        ora_contracts::MarketplaceArtifactRetrievalUpdate::DirectHttps,
                },
                5,
            )
            .expect("switch back to Direct HTTPS");
        let records = SqlitePluginMarketplaceSourceRepository::new(pool)
            .list_sources()
            .expect("reload Direct HTTPS source");
        assert_eq!(records[1].artifact_retrieval, r#"{"type":"direct_https"}"#);
    }

    #[test]
    fn update_replaces_the_git_url_in_place() {
        let temp = TempDir::new().expect("create temp directory");
        let (_, store) = store_with_repository(&temp);
        let original = third_party("https://github.com/example/original");
        store.add(original.clone(), 2).expect("add source");

        let renamed = MarketplaceSource {
            url: "https://github.com/example/renamed".to_owned(),
            branch: "main".to_owned(),
            use_proxy: false,
            enabled: true,
            artifact_retrieval: MarketplaceArtifactRetrieval::DirectHttps,
        };
        let sources = store
            .update(
                UpdateMarketplaceSourceRequest {
                    url: original.url.clone(),
                    new_url: renamed.url.clone(),
                    branch: renamed.branch.clone(),
                    use_proxy: renamed.use_proxy,
                    enabled: renamed.enabled,
                    artifact_retrieval:
                        ora_contracts::MarketplaceArtifactRetrievalUpdate::DirectHttps,
                },
                3,
            )
            .expect("rename source");

        assert!(sources.contains(&renamed));
        assert!(!sources.iter().any(|source| source.url == original.url));
    }

    #[test]
    fn update_rejects_a_second_spelling_of_another_source() {
        let temp = TempDir::new().expect("create temp directory");
        let (_, store) = store_with_repository(&temp);
        let first = third_party("https://github.com/example/first");
        let second = third_party("https://github.com/example/second");
        store.add(first.clone(), 2).expect("add first");
        store.add(second.clone(), 3).expect("add second");

        assert!(matches!(
            store.update(
                UpdateMarketplaceSourceRequest {
                    url: second.url.clone(),
                    new_url: "https://github.com/example/first.git".to_owned(),
                    branch: "main".to_owned(),
                    use_proxy: false,
                    enabled: true,
                    artifact_retrieval:
                        ora_contracts::MarketplaceArtifactRetrievalUpdate::DirectHttps,
                },
                4,
            ),
            Err(MarketplaceSourceStoreError::Duplicate(_))
        ));
    }

    #[test]
    fn appends_after_deleting_an_interior_source() {
        let temp = TempDir::new().expect("create temp directory");
        let (_, store) = store_with_repository(&temp);
        let first = third_party("https://github.com/example/first");
        let second = third_party("https://github.com/example/second");
        store.add(first.clone(), 2).expect("add first source");
        store.add(second.clone(), 3).expect("add second source");
        store.delete(&first.url).expect("delete interior source");

        let third = third_party("https://github.com/example/third");
        let sources = store
            .add(third.clone(), 4)
            .expect("append after interior delete");

        assert!(sources.contains(&third));
        assert_eq!(sources.len(), 3);
    }

    #[test]
    fn duplicate_and_missing_sources_are_rejected() {
        let temp = TempDir::new().expect("create temp directory");
        let (_, store) = store_with_repository(&temp);

        assert!(matches!(
            store.add(
                MarketplaceSource {
                    url: DEFAULT_MARKETPLACE_URL.to_owned(),
                    branch: DEFAULT_MARKETPLACE_BRANCH.to_owned(),
                    use_proxy: false,
                    enabled: true,
                    artifact_retrieval: MarketplaceArtifactRetrieval::DirectHttps,
                },
                2,
            ),
            Err(MarketplaceSourceStoreError::Duplicate(_))
        ));
        assert!(matches!(
            store.delete("https://github.com/missing/marketplace"),
            Err(MarketplaceSourceStoreError::NotFound(_))
        ));
    }

    /// Verifies the default marketplace keeps the reserved readable namespace while a third-party
    /// source is given a derived one that can never collide with it.
    #[test]
    fn binds_the_reserved_namespace_only_to_the_default_source() {
        let temp = TempDir::new().expect("create temp directory");
        let (_, store) = store_with_repository(&temp);
        let third_party = third_party("https://github.com/acme/plugins");
        store.add(third_party.clone(), 2).expect("add source");

        let default_source = store
            .registry_source(
                &MarketplaceSource {
                    url: DEFAULT_MARKETPLACE_URL.to_owned(),
                    branch: DEFAULT_MARKETPLACE_BRANCH.to_owned(),
                    use_proxy: false,
                    enabled: true,
                    artifact_retrieval: MarketplaceArtifactRetrieval::DirectHttps,
                },
                3,
            )
            .expect("bind default source");
        let derived_source = store
            .registry_source(&third_party, 3)
            .expect("bind third-party source");

        assert_eq!(
            (
                default_source.namespace().as_str().to_owned(),
                derived_source.namespace().as_str().to_owned(),
                derived_source.namespace().is_reserved(),
            ),
            (
                "official".to_string(),
                "plugins.136aca80".to_string(),
                false,
            ),
        );
    }

    /// Verifies equivalent spellings of one repository resolve to a single namespace, and that a
    /// source removed and added back under a different spelling reuses its original binding.
    ///
    /// This is the property that keeps an installed plugin attached to its source. The install
    /// path froze the namespace when the package landed, so a binding that drifted — because the
    /// user retyped the URL with a `.git` suffix, or because normalization learned a new
    /// equivalence in a later version — would leave the marketplace listing the plugin under an id
    /// no installed package answers to: it would show as not installed, refuse to update, and
    /// install a second copy.
    #[test]
    fn reuses_one_binding_across_equivalent_urls_and_re_adds() {
        let temp = TempDir::new().expect("create temp directory");
        let pool = open_pool(&temp);
        let store = store_with_pool(&temp, pool.clone());
        let original = third_party("https://github.com/acme/plugins");
        store.add(original.clone(), 2).expect("add source");
        let bound = store
            .registry_source(&original, 3)
            .expect("bind source")
            .namespace()
            .clone();

        store.delete(&original.url).expect("delete source");
        let equivalents = [
            "https://GitHub.com/Acme/Plugins",
            "https://github.com/acme/plugins.git",
            "https://github.com:443/acme/plugins/",
        ];
        let namespaces = equivalents.map(|url| {
            store
                .registry_source(&third_party(url), 4)
                .expect("re-bind an equivalent spelling")
                .namespace()
                .clone()
        });

        // A later version that recognizes a new equivalent spelling must resolve it to the stored
        // binding too, never derive a second identity for the same repository.
        assert_eq!(
            namespaces.to_vec(),
            vec![bound.clone(), bound.clone(), bound.clone()],
        );
        // Re-adding the repository reuses the row rather than writing a second one.
        store
            .add(third_party("https://github.com/acme/plugins.git"), 5)
            .expect("re-add the source");
        assert_eq!(
            store
                .registry_source(&original, 6)
                .expect("bind after re-add")
                .namespace(),
            &bound,
        );
    }

    /// Verifies one repository cannot be configured twice through two spellings, which would
    /// otherwise list one source as two entries sharing a namespace and a checkout.
    #[test]
    fn rejects_a_second_spelling_of_a_configured_source() {
        let temp = TempDir::new().expect("create temp directory");
        let (_, store) = store_with_repository(&temp);
        store
            .add(third_party("https://github.com/acme/plugins"), 2)
            .expect("add source");

        assert!(matches!(
            store.add(third_party("https://GitHub.com/Acme/Plugins.git/"), 3),
            Err(MarketplaceSourceStoreError::Duplicate(_))
        ));
    }
}
