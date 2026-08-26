use ora_contracts::MarketplaceSource;
use ora_db::{PluginMarketplaceSourceRecord, SqlitePluginMarketplaceSourceRepository};
use ora_plugin_registry::{RegistryError, RegistrySource};
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
}

/// Owns SQLite-backed marketplace source configuration and binds each row to a checkout path.
pub(crate) struct MarketplaceSourceStore {
    repository: SqlitePluginMarketplaceSourceRepository,
    sources_root: PathBuf,
}

impl MarketplaceSourceStore {
    /// Loads existing sources or seeds the default source when the table is empty.
    pub(crate) fn open(
        repository: SqlitePluginMarketplaceSourceRepository,
        data_directory: &Path,
        now_ms: i64,
    ) -> Result<Self, MarketplaceSourceStoreError> {
        let sources_root = data_directory.join("plugins").join("sources");
        let store = Self {
            repository,
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
        if current.iter().any(|existing| existing.url == source.url) {
            return Err(MarketplaceSourceStoreError::Duplicate(source.url));
        }
        checked_source(&source, &self.sources_root)?;
        let position = current
            .iter()
            .map(|existing| existing.position)
            .max()
            .map_or(0, |highest| highest + 1);
        self.insert(source, position, now_ms)?;
        self.list()
    }

    /// Removes one source by URL and returns the remaining sources.
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

    /// Binds one contract-shaped source to a validated [`RegistrySource`] in the source tree.
    pub(crate) fn registry_source(
        &self,
        source: &MarketplaceSource,
    ) -> Result<RegistrySource, MarketplaceSourceStoreError> {
        checked_source(source, &self.sources_root).map_err(MarketplaceSourceStoreError::Validation)
    }

    /// Inserts one already-validated source at the supplied precedence position.
    fn insert(
        &self,
        source: MarketplaceSource,
        position: i64,
        now_ms: i64,
    ) -> Result<(), MarketplaceSourceStoreError> {
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

/// Validates and binds one wire source to its derived checkout directory.
fn checked_source(
    source: &MarketplaceSource,
    sources_root: &Path,
) -> Result<RegistrySource, RegistryError> {
    RegistrySource::try_from_git(source.url.clone(), source.branch.clone(), sources_root)
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
    use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn store_with_repository(temp: &TempDir) -> (RepositoryStoreGuard, MarketplaceSourceStore) {
        let database_path = temp.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("open repository pool");
        let store = MarketplaceSourceStore::open(
            SqlitePluginMarketplaceSourceRepository::new(pool),
            temp.path(),
            1,
        )
        .expect("open store");
        (RepositoryStoreGuard, store)
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
        let first = MarketplaceSource {
            url: "https://github.com/example/first".to_owned(),
            branch: "main".to_owned(),
            use_proxy: false,
        };
        let second = MarketplaceSource {
            url: "https://github.com/example/second".to_owned(),
            branch: "main".to_owned(),
            use_proxy: false,
        };
        store.add(first.clone(), 2).expect("add first source");
        store.add(second.clone(), 3).expect("add second source");
        store.delete(&first.url).expect("delete interior source");

        let third = MarketplaceSource {
            url: "https://github.com/example/third".to_owned(),
            branch: "main".to_owned(),
            use_proxy: false,
        };
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
}
