use ora_contracts::MarketplaceSource;
use ora_plugin_registry::{RegistryError, RegistrySource};
use ora_utils::atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use thiserror::Error;

/// The filename for the persisted, user-editable marketplace source list.
const MARKETPLACE_SOURCES_FILENAME: &str = "marketplace_sources.json";

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
    #[error("marketplace source configuration file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("marketplace source configuration JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// On-disk container for every marketplace source, kept under the plugin data root.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedSources {
    sources: Vec<MarketplaceSource>,
}

/// Owns the runtime marketplace source list and its small configuration file.
///
/// `PluginApi` only needs to query `RegistrySource` snapshots, while add/delete are serialized
/// through the store so a source change is atomic with respect to the persisted configuration.
pub(crate) struct MarketplaceSourceStore {
    config_path: PathBuf,
    sources_root: PathBuf,
    sources: RwLock<Vec<RegistrySource>>,
}

impl MarketplaceSourceStore {
    /// Loads existing sources or writes the default source when no configuration file exists yet.
    pub(crate) fn open(data_directory: &Path) -> Result<Self, MarketplaceSourceStoreError> {
        let plugins_directory = data_directory.join("plugins");
        let config_path = plugins_directory.join(MARKETPLACE_SOURCES_FILENAME);
        let sources_root = plugins_directory.join("sources");

        let sources = if config_path.exists() {
            let bytes = fs::read(&config_path)?;
            let persisted: PersistedSources = serde_json::from_slice(&bytes)?;
            persisted
                .sources
                .into_iter()
                .map(|source| checked_source(source, &sources_root))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let default_source = checked_source(
                MarketplaceSource {
                    url: DEFAULT_MARKETPLACE_URL.to_owned(),
                    branch: DEFAULT_MARKETPLACE_BRANCH.to_owned(),
                },
                &sources_root,
            )?;
            let store = Self {
                config_path,
                sources_root,
                sources: RwLock::new(vec![default_source]),
            };
            store.save()?;
            return Ok(store);
        };

        Ok(Self {
            config_path,
            sources_root,
            sources: RwLock::new(sources),
        })
    }

    /// Returns the current source list in source-precedence order.
    pub(crate) fn list(&self) -> Vec<MarketplaceSource> {
        self.read_sources().iter().map(source_spec).collect()
    }

    /// Returns a cloned runtime source snapshot for sync and install operations.
    pub(crate) fn snapshot(&self) -> Vec<RegistrySource> {
        self.read_sources().clone()
    }

    /// Validates, stores, and persists one additional source, then returns the new ordering.
    pub(crate) fn add(
        &self,
        source: MarketplaceSource,
    ) -> Result<Vec<MarketplaceSource>, MarketplaceSourceStoreError> {
        let source = checked_source(source, &self.sources_root)?;
        let mut sources = self.write_sources();
        if sources
            .iter()
            .any(|existing| existing.url() == source.url())
        {
            return Err(MarketplaceSourceStoreError::Duplicate(
                source.url().to_owned(),
            ));
        }
        sources.push(source);
        self.save_lock(&sources)?;
        Ok(sources.iter().map(source_spec).collect())
    }

    /// Removes one source by URL, persists the new ordering, and returns the remaining sources.
    pub(crate) fn delete(
        &self,
        url: &str,
    ) -> Result<Vec<MarketplaceSource>, MarketplaceSourceStoreError> {
        let mut sources = self.write_sources();
        let previous_len = sources.len();
        sources.retain(|source| source.url() != url);
        if sources.len() == previous_len {
            return Err(MarketplaceSourceStoreError::NotFound(url.to_owned()));
        }
        self.save_lock(&sources)?;
        Ok(sources.iter().map(source_spec).collect())
    }

    /// Persists the current in-memory list while holding the read lock.
    fn save(&self) -> Result<(), MarketplaceSourceStoreError> {
        let sources = self.read_sources();
        self.save_lock(&sources)
    }

    /// Atomically writes one source slice, creating the parent directory when needed.
    fn save_lock(&self, sources: &[RegistrySource]) -> Result<(), MarketplaceSourceStoreError> {
        let persisted = PersistedSources {
            sources: sources.iter().map(source_spec).collect(),
        };
        let bytes = serde_json::to_vec(&persisted)?;
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic::write(&self.config_path, &bytes)?;
        Ok(())
    }

    fn read_sources(&self) -> RwLockReadGuard<'_, Vec<RegistrySource>> {
        self.sources.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_sources(&self) -> RwLockWriteGuard<'_, Vec<RegistrySource>> {
        self.sources.write().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Validates and binds one wire source to its derived checkout directory.
fn checked_source(
    source: MarketplaceSource,
    sources_root: &Path,
) -> Result<RegistrySource, MarketplaceSourceStoreError> {
    RegistrySource::try_from_git(source.url, source.branch, sources_root)
        .map_err(MarketplaceSourceStoreError::Validation)
}

/// Projects one runtime source back to the frontend-facing wire shape.
fn source_spec(source: &RegistrySource) -> MarketplaceSource {
    MarketplaceSource {
        url: source.url().to_owned(),
        branch: source.branch().as_str().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn missing_config_seeds_the_default_source() {
        let temp = TempDir::new().expect("create temp directory");
        let store = MarketplaceSourceStore::open(temp.path()).expect("open store");

        assert_eq!(
            store.list(),
            vec![MarketplaceSource {
                url: DEFAULT_MARKETPLACE_URL.to_owned(),
                branch: DEFAULT_MARKETPLACE_BRANCH.to_owned(),
            }]
        );
    }

    #[test]
    fn add_and_delete_persist_and_return_current_sources() {
        let temp = TempDir::new().expect("create temp directory");
        let store = MarketplaceSourceStore::open(temp.path()).expect("open store");
        let added = MarketplaceSource {
            url: "https://github.com/example/marketplace".to_owned(),
            branch: "main".to_owned(),
        };

        let sources = store.add(added.clone()).expect("add source");
        assert_eq!(sources.len(), 2);
        assert!(sources.contains(&added));
        assert!(store.config_path.parent().is_some_and(|path| path.exists()));
        assert!(store.config_path.exists());

        let reloaded = MarketplaceSourceStore::open(temp.path()).expect("reload store");
        assert_eq!(reloaded.list(), sources);

        let remaining = store.delete(&added.url).expect("delete source");
        assert_eq!(remaining.len(), 1);
        assert!(
            !store
                .snapshot()
                .iter()
                .any(|source| source.url() == added.url)
        );
    }

    #[test]
    fn duplicate_and_missing_sources_are_rejected() {
        let temp = TempDir::new().expect("create temp directory");
        let store = MarketplaceSourceStore::open(temp.path()).expect("open store");
        let duplicate = MarketplaceSource {
            url: DEFAULT_MARKETPLACE_URL.to_owned(),
            branch: DEFAULT_MARKETPLACE_BRANCH.to_owned(),
        };

        assert!(matches!(
            store.add(duplicate),
            Err(MarketplaceSourceStoreError::Duplicate(_))
        ));
        assert!(matches!(
            store.delete("https://github.com/missing/marketplace"),
            Err(MarketplaceSourceStoreError::NotFound(_))
        ));
    }

    #[test]
    fn rejects_invalid_source_input_before_persisting() {
        let temp = TempDir::new().expect("create temp directory");
        let store = MarketplaceSourceStore::open(temp.path()).expect("open store");

        assert!(matches!(
            store.add(MarketplaceSource {
                url: "http://github.com/example/marketplace".to_owned(),
                branch: "main".to_owned(),
            }),
            Err(MarketplaceSourceStoreError::Validation(_))
        ));
    }
}
