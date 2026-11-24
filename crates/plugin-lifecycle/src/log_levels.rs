//! Host-owned log levels preserve unknown persisted entries and expose write protection.

use ora_domain::PluginId;
use ora_logging::{LogLevel, ora_warn};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use thiserror::Error;
use tokio::sync::watch;

/// Level used when this plugin has no readable explicit preference.
pub const DEFAULT_PLUGIN_LOG_LEVEL: LogLevel = LogLevel::Info;
type RawLevels = BTreeMap<String, Box<RawValue>>;

/// The effective value plus the file that needs repair before ordinary edits are safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLogLevelState {
    pub level: LogLevel,
    pub configured: bool,
    pub read_only_path: Option<PathBuf>,
}

/// Distinguishes protected old data from an ordinary filesystem write failure.
#[derive(Debug, Error)]
pub enum PluginLogLevelPersistError {
    #[error(
        "plugin log levels at `{path}` are read-only; repair the file and restart before editing"
    )]
    ReadOnly { path: PathBuf },
    #[error("failed to persist plugin log levels at `{path}`: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Registry of persisted Plugin log levels and live subscriptions, keyed by canonical identity.
#[derive(Clone)]
pub struct PluginLogLevels {
    inner: Arc<Mutex<Inner>>,
    path: PathBuf,
}

struct Inner {
    configured: BTreeMap<PluginId, LogLevel>,
    // Raw values keep even future JSON numbers/objects byte-for-byte when an explicit uninstall
    // removes one key. None means the file could not be parsed, so no destructive edit is safe.
    raw: Option<RawLevels>,
    persisted: Option<Vec<u8>>,
    writable: bool,
    publishers: BTreeMap<PluginId, watch::Sender<LogLevel>>,
}

/// Projects known levels without discarding the original JSON or silently hiding protection.
fn known_levels(path: &Path, raw: &RawLevels) -> (BTreeMap<PluginId, LogLevel>, bool) {
    let mut writable = true;
    let configured = raw.iter().filter_map(|(key, value)| {
        let id = match PluginId::parse(key) {
            Ok(id) => id,
            Err(_) => {
                writable = false;
                ora_warn!(path = %path.display(), plugin_id = %key, "plugin log level identity is invalid; settings are read-only");
                return None;
            }
        };
        match serde_json::from_str::<LogLevel>(value.get()) {
            Ok(level) => Some((id, level)),
            Err(_) => {
                writable = false;
                ora_warn!(path = %path.display(), plugin_id = %id, "plugin log level is unsupported; settings are read-only");
                None
            }
        }
    }).collect();
    (configured, writable)
}

impl PluginLogLevels {
    /// Missing is unset; unreadable or unsupported persisted data remains protected until repair.
    pub fn open(data_directory: &Path) -> Self {
        let path = data_directory.join("plugins").join("log-levels.json");
        let mut persisted = None;
        let raw = match std::fs::read(&path) {
            Ok(bytes) => {
                persisted = Some(bytes.clone());
                match serde_json::from_slice::<RawLevels>(&bytes) {
                    Ok(raw) => Some(raw),
                    Err(error) => {
                        ora_warn!(path = %path.display(), %error, "plugin log levels file is invalid; settings are read-only");
                        None
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Some(BTreeMap::new()),
            Err(error) => {
                ora_warn!(path = %path.display(), %error, "plugin log levels file could not be read; settings are read-only");
                None
            }
        };
        let (configured, writable) = raw
            .as_ref()
            .map(|raw| known_levels(&path, raw))
            .unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(Inner {
                configured,
                raw,
                persisted,
                writable,
                publishers: BTreeMap::new(),
            })),
            path,
        }
    }

    /// Reports the effective level and a repair path whenever writes would destroy old data.
    pub fn state(&self, plugin_id: &PluginId) -> PluginLogLevelState {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        PluginLogLevelState {
            level: inner
                .configured
                .get(plugin_id)
                .copied()
                .unwrap_or(DEFAULT_PLUGIN_LOG_LEVEL),
            configured: inner.configured.contains_key(plugin_id),
            read_only_path: (!inner.writable).then(|| self.path.clone()),
        }
    }

    /// Subscribes a generation to its next effective log level without exposing host settings.
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

    /// Persists first, then publishes; failed or protected writes leave every view unchanged.
    pub fn set(
        &self,
        plugin_id: &PluginId,
        level: LogLevel,
    ) -> Result<PluginLogLevelState, PluginLogLevelPersistError> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if !inner.writable {
            return Err(PluginLogLevelPersistError::ReadOnly {
                path: self.path.clone(),
            });
        }
        let mut next = inner
            .raw
            .clone()
            .ok_or_else(|| PluginLogLevelPersistError::ReadOnly {
                path: self.path.clone(),
            })?;
        next.insert(
            plugin_id.to_string(),
            serde_json::value::to_raw_value(&level).map_err(|error| {
                PluginLogLevelPersistError::Write {
                    path: self.path.clone(),
                    source: io::Error::other(error),
                }
            })?,
        );
        self.persist(&mut inner, &next)?;
        inner.raw = Some(next);
        inner.configured.insert(plugin_id.clone(), level);
        if let Some(publisher) = inner.publishers.get(plugin_id) {
            publisher.send_replace(level);
        }
        Ok(PluginLogLevelState {
            level,
            configured: true,
            read_only_path: None,
        })
    }

    /// Explicit delete-data uninstall removes only this identity, preserving all other raw values.
    /// Malformed or unreadable files cannot be safely edited, even for a currently unknown key.
    pub fn clear(&self, plugin_id: &PluginId) -> Result<(), PluginLogLevelPersistError> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = inner
            .raw
            .clone()
            .ok_or_else(|| PluginLogLevelPersistError::ReadOnly {
                path: self.path.clone(),
            })?;
        if next.remove(&plugin_id.to_string()).is_none() {
            return Ok(());
        }
        self.persist(&mut inner, &next)?;
        let (configured, writable) = known_levels(&self.path, &next);
        inner.configured = configured;
        inner.writable = writable;
        inner.raw = Some(next);
        if let Some(publisher) = inner.publishers.get(plugin_id) {
            publisher.send_replace(DEFAULT_PLUGIN_LOG_LEVEL);
        }
        Ok(())
    }

    /// Atomically replaces the map while retaining unsupported JSON values verbatim.
    fn persist(
        &self,
        inner: &mut Inner,
        raw: &RawLevels,
    ) -> Result<(), PluginLogLevelPersistError> {
        let write_error = |source| PluginLogLevelPersistError::Write {
            path: self.path.clone(),
            source,
        };
        let bytes =
            serde_json::to_vec_pretty(raw).map_err(|error| write_error(io::Error::other(error)))?;
        let current = match std::fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(write_error(error)),
        };
        if current != inner.persisted {
            inner.writable = false;
            return Err(PluginLogLevelPersistError::ReadOnly {
                path: self.path.clone(),
            });
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(write_error)?;
        }
        ora_utils::atomic::write(&self.path, &bytes).map_err(write_error)?;
        inner.persisted = Some(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
