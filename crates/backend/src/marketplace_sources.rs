use ora_contracts::MarketplaceSource;
use ora_db::{
    PluginMarketplaceSourceRecord, SqlitePluginMarketplaceSourceRepository,
    SqlitePluginSourceNamespaceRepository,
};
use ora_domain::{PluginIdError, PluginNamespace};
use ora_plugin_registry::{RegistryError, RegistrySource};
use ora_utils::url::canonical_repository_url;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// The seed source used the first time a backend opens before any user configuration exists.
const DEFAULT_MARKETPLACE_URL: &str = "https://github.com/ora-space/marketplace";
const DEFAULT_MARKETPLACE_BRANCH: &str = "main";

/// Reports failures while loading, validating, or persisting the marketplace source list.
#[derive(Debug, Error)]
pub(crate) enum MarketplaceSourceStoreError {
    #[error("invalid marketplace source: {0}")]
    Validation(#[from] RegistryError),
    #[error("marketplace source already exists: {0}")]
    Duplicate(String),
    #[error("marketplace source was not found: {0}")]
    NotFound(String),
    #[error("marketplace source repository operation failed: {0}")]
    Repository(#[from] ora_db::DatabaseError),
    /// A persisted binding holds a namespace this version cannot represent, so the source cannot
    /// be used without either inventing a new identity for it or silently changing an existing
    /// one — both of which would detach its already-installed plugins.
    #[error("persisted marketplace source namespace is unusable: {0}")]
    CorruptNamespace(#[from] PluginIdError),
}

/// Owns SQLite-backed marketplace source configuration and binds each row to a namespace and a
/// checkout path.
pub(crate) struct MarketplaceSourceStore {
    repository: SqlitePluginMarketplaceSourceRepository,
    namespaces: SqlitePluginSourceNamespaceRepository,
    sources_root: PathBuf,
}

impl MarketplaceSourceStore {
    /// Loads existing sources or seeds the default source when the table is empty.
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
        if store.repository.list_sources()?.is_empty() {
            let default_source = MarketplaceSource {
                url: DEFAULT_MARKETPLACE_URL.to_owned(),
                branch: DEFAULT_MARKETPLACE_BRANCH.to_owned(),
                use_proxy: false,
            };
            store.insert(default_source, /*position*/ 0, now_ms)?;
        }
        Ok(store)
    }

    /// Returns the current source list in source-precedence order.
    pub(crate) fn list(&self) -> Result<Vec<MarketplaceSource>, MarketplaceSourceStoreError> {
        let sources = self.repository.list_sources()?;
        Ok(sources.iter().map(source_spec).collect())
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
        self.insert(source, position, now_ms)?;
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

    /// Changes only the proxy policy of one source and returns the authoritative list.
    pub(crate) fn set_use_proxy(
        &self,
        url: &str,
        use_proxy: bool,
        now_ms: i64,
    ) -> Result<Vec<MarketplaceSource>, MarketplaceSourceStoreError> {
        if !self.repository.set_use_proxy(url, use_proxy, now_ms)? {
            return Err(MarketplaceSourceStoreError::NotFound(url.to_owned()));
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
fn source_spec(source: &PluginMarketplaceSourceRecord) -> MarketplaceSource {
    MarketplaceSource {
        url: source.url.clone(),
        branch: source.branch.clone(),
        use_proxy: source.use_proxy,
    }
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
        let database_path = temp.path().join("ora.sqlite3");
        DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("open repository pool")
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
            }]
        );
    }

    #[test]
    fn add_update_and_delete_persist_sources() {
        let temp = TempDir::new().expect("create temp directory");
        let (_, store) = store_with_repository(&temp);
        let added = MarketplaceSource {
            url: "https://github.com/example/marketplace".to_owned(),
            branch: "main".to_owned(),
            use_proxy: true,
        };

        let sources = store.add(added.clone(), 2).expect("add source");
        assert_eq!(sources.len(), 2);
        assert!(sources.contains(&added));

        let updated = store
            .set_use_proxy(&added.url, false, 3)
            .expect("update source");
        assert_eq!(updated[1].use_proxy, false);

        let remaining = store.delete(&added.url).expect("delete source");
        assert_eq!(remaining.len(), 1);
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
