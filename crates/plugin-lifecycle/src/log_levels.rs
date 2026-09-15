//! Host-owned, persisted Plugin log level per canonical Plugin ID.
//!
//! The setting decides the lowest level the host persists for one plugin's diagnostics. It is
//! keyed by identity rather than by version so upgrades keep it, it lives in a host file outside
//! every plugin data directory so `ora/storage/*` can never reach it, and it is published to
//! running generations through a `watch` channel so a change applies to the next record the
//! reader filters — no restart, no SDK involvement.

use ora_domain::PluginId;
use ora_logging::{LogLevel, ora_warn};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use thiserror::Error;
use tokio::sync::watch;

/// Level applied to every plugin without an explicit setting.
pub const DEFAULT_PLUGIN_LOG_LEVEL: LogLevel = LogLevel::Info;

const PLUGINS_DIRECTORY: &str = "plugins";
const LOG_LEVELS_FILE_NAME: &str = "log-levels.json";

/// The effective level of one plugin and whether it came from an explicit setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginLogLevelState {
    pub level: LogLevel,
    pub configured: bool,
}

/// Why a level could not be persisted or cleared; the in-memory level is untouched on error.
#[derive(Debug, Error)]
#[error("failed to persist plugin log levels at `{path}`")]
pub struct PluginLogLevelPersistError {
    pub path: PathBuf,
    #[source]
    pub source: io::Error,
}

/// Registry of per-plugin log levels with atomic persistence and live subscriptions.
#[derive(Clone)]
pub struct PluginLogLevels {
    inner: Arc<Mutex<Inner>>,
    path: PathBuf,
}

struct Inner {
    /// The persisted settings; absence means the default applies.
    configured: BTreeMap<PluginId, LogLevel>,
    /// One publisher per plugin that has ever been subscribed to or configured.
    publishers: BTreeMap<PluginId, watch::Sender<LogLevel>>,
}

impl PluginLogLevels {
    /// Loads `<data-dir>/plugins/log-levels.json`, treating an absent file as "nothing set".
    ///
    /// An unreadable or unparsable file is reported and treated as empty: starting with every
    /// plugin at the default is a safe state, and the next successful write replaces the file.
    pub fn open(data_directory: &Path) -> Self {
        let path = data_directory
            .join(PLUGINS_DIRECTORY)
            .join(LOG_LEVELS_FILE_NAME);
        let configured = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<BTreeMap<String, LogLevel>>(&bytes) {
                Ok(raw) => raw
                    .into_iter()
                    .filter_map(|(id, level)| match PluginId::parse(&id) {
                        Ok(id) => Some((id, level)),
                        Err(_) => {
                            ora_warn!(
                                path = %path.display(),
                                plugin_id = %id,
                                "ignoring plugin log level entry with an invalid plugin id"
                            );
                            None
                        }
                    })
                    .collect(),
                Err(error) => {
                    ora_warn!(
                        path = %path.display(),
                        %error,
                        "plugin log levels file is unreadable; every plugin uses the default"
                    );
                    BTreeMap::new()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => {
                ora_warn!(
                    path = %path.display(),
                    %error,
                    "plugin log levels file could not be read; every plugin uses the default"
                );
                BTreeMap::new()
            }
        };
        Self {
            inner: Arc::new(Mutex::new(Inner {
                configured,
                publishers: BTreeMap::new(),
            })),
            path,
        }
    }

    /// Returns the effective level of one plugin and whether it is explicitly configured.
    pub fn state(&self, plugin_id: &PluginId) -> PluginLogLevelState {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match inner.configured.get(plugin_id) {
            Some(level) => PluginLogLevelState {
                level: *level,
                configured: true,
            },
            None => PluginLogLevelState {
                level: DEFAULT_PLUGIN_LOG_LEVEL,
                configured: false,
            },
        }
    }

    /// Subscribes a process generation to the plugin's live effective level.
    pub fn subscribe(&self, plugin_id: &PluginId) -> watch::Receiver<LogLevel> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let level = inner
            .configured
            .get(plugin_id)
            .copied()
            .unwrap_or(DEFAULT_PLUGIN_LOG_LEVEL);
        inner
            .publishers
            .entry(plugin_id.clone())
            .or_insert_with(|| watch::channel(level).0)
            .subscribe()
    }

    /// Persists a new level and only then publishes it to running generations.
    ///
    /// Persist-then-apply keeps the two views consistent: a failed write leaves both the file
    /// and the effective level as they were, so the caller can truthfully report failure.
    pub fn set(
        &self,
        plugin_id: &PluginId,
        level: LogLevel,
    ) -> Result<PluginLogLevelState, PluginLogLevelPersistError> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = inner.configured.clone();
        next.insert(plugin_id.clone(), level);
        self.persist(&next)?;
        inner.configured = next;
        if let Some(publisher) = inner.publishers.get(plugin_id) {
            publisher.send_replace(level);
        }
        Ok(PluginLogLevelState {
            level,
            configured: true,
        })
    }

    /// Removes the plugin's setting so a reinstall under the same identity starts at default.
    pub fn clear(&self, plugin_id: &PluginId) -> Result<(), PluginLogLevelPersistError> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if !inner.configured.contains_key(plugin_id) {
            return Ok(());
        }
        let mut next = inner.configured.clone();
        next.remove(plugin_id);
        self.persist(&next)?;
        inner.configured = next;
        if let Some(publisher) = inner.publishers.get(plugin_id) {
            publisher.send_replace(DEFAULT_PLUGIN_LOG_LEVEL);
        }
        Ok(())
    }

    /// Atomically replaces the settings file with the given map.
    fn persist(
        &self,
        configured: &BTreeMap<PluginId, LogLevel>,
    ) -> Result<(), PluginLogLevelPersistError> {
        let raw = configured
            .iter()
            .map(|(id, level)| (id.to_string(), *level))
            .collect::<BTreeMap<_, _>>();
        let bytes =
            serde_json::to_vec_pretty(&raw).map_err(|error| PluginLogLevelPersistError {
                path: self.path.clone(),
                source: io::Error::other(error),
            })?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| PluginLogLevelPersistError {
                path: self.path.clone(),
                source,
            })?;
        }
        ora_utils::atomic::write(&self.path, &bytes).map_err(|source| PluginLogLevelPersistError {
            path: self.path.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PLUGIN_LOG_LEVEL, PluginLogLevelState, PluginLogLevels};
    use ora_domain::PluginId;
    use ora_logging::LogLevel;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn id(name: &str) -> PluginId {
        PluginId::new("official", name).expect("plugin id")
    }

    /// Unset plugins report the default; a set level persists across reopen, publishes to live
    /// subscribers, and leaves other plugins untouched.
    #[test]
    fn levels_are_per_plugin_persisted_and_published_live() {
        let temp = TempDir::new().expect("temp dir");
        let levels = PluginLogLevels::open(temp.path());
        let a_live = levels.subscribe(&id("a"));
        let b_live = levels.subscribe(&id("b"));
        assert_eq!(
            (levels.state(&id("a")), *a_live.borrow()),
            (
                PluginLogLevelState {
                    level: DEFAULT_PLUGIN_LOG_LEVEL,
                    configured: false
                },
                LogLevel::Info
            )
        );

        levels.set(&id("a"), LogLevel::Debug).expect("set a");

        let reopened = PluginLogLevels::open(temp.path());
        assert_eq!(
            (
                *a_live.borrow(),
                *b_live.borrow(),
                levels.state(&id("b")),
                reopened.state(&id("a")),
                reopened.state(&id("b")),
            ),
            (
                LogLevel::Debug,
                LogLevel::Info,
                PluginLogLevelState {
                    level: LogLevel::Info,
                    configured: false
                },
                PluginLogLevelState {
                    level: LogLevel::Debug,
                    configured: true
                },
                PluginLogLevelState {
                    level: LogLevel::Info,
                    configured: false
                },
            )
        );
    }

    /// A failed persist reports an error and changes neither the effective nor the stored level.
    #[test]
    fn a_failed_persist_changes_nothing() {
        let temp = TempDir::new().expect("temp dir");
        let levels = PluginLogLevels::open(temp.path());
        levels.set(&id("a"), LogLevel::Warn).expect("initial set");
        let live = levels.subscribe(&id("a"));
        // Occupying the parent path with a file makes the atomic replace fail.
        let plugins_dir = temp.path().join("plugins");
        std::fs::remove_dir_all(&plugins_dir).expect("remove plugins dir");
        std::fs::write(&plugins_dir, "not a directory").expect("occupy path");

        let error = levels
            .set(&id("a"), LogLevel::Trace)
            .err()
            .expect("persist fails");

        assert_eq!(
            (error.path, levels.state(&id("a")), *live.borrow(),),
            (
                plugins_dir.join("log-levels.json"),
                PluginLogLevelState {
                    level: LogLevel::Warn,
                    configured: true
                },
                LogLevel::Warn,
            )
        );
    }

    /// Clearing removes the entry, publishes the default, and is a no-op for unset plugins.
    #[test]
    fn clear_restores_the_default_and_tolerates_unset_plugins() {
        let temp = TempDir::new().expect("temp dir");
        let levels = PluginLogLevels::open(temp.path());
        let live = levels.subscribe(&id("a"));
        levels.set(&id("a"), LogLevel::Error).expect("set");
        levels.clear(&id("a")).expect("clear");
        levels.clear(&id("never-set")).expect("clear unset");
        assert_eq!(
            (
                *live.borrow(),
                PluginLogLevels::open(temp.path()).state(&id("a")),
            ),
            (
                LogLevel::Info,
                PluginLogLevelState {
                    level: LogLevel::Info,
                    configured: false
                }
            )
        );
    }
}
