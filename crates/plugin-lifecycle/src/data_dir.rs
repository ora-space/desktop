//! Owns the per-plugin writable data directory and the host-managed plugin log directory below
//! the Ora data root.
//!
//! Installed packages are read-only; the data directory is the only place a plugin may write,
//! and both trees survive version upgrades because they are keyed by plugin identity, not by
//! installed version. The directory levels are the id's namespace and name, which manifest
//! validation already bounds to slug segments, so they are safe on every platform without
//! further escaping.

use ora_domain::PluginId;
use std::io;
use std::path::{Path, PathBuf};

const PLUGINS_DIRECTORY: &str = "plugins";
const DATA_DIRECTORY: &str = "data";
const DOWNLOADS_DIRECTORY: &str = "downloads";
const LOGS_DIRECTORY: &str = "logs";

/// Creates and locates `<data-dir>/plugins/data/<namespace>/<name>/` directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDataDirectories {
    root: PathBuf,
}

impl PluginDataDirectories {
    /// Anchors plugin data below the same data directory that holds installed packages.
    pub fn new(data_directory: impl Into<PathBuf>) -> Self {
        Self {
            root: data_directory
                .into()
                .join(PLUGINS_DIRECTORY)
                .join(DATA_DIRECTORY),
        }
    }

    /// Returns the plugin's data directory without touching the filesystem.
    pub fn path_for(&self, plugin_id: &PluginId) -> PathBuf {
        self.root.join(plugin_id.namespace()).join(plugin_id.name())
    }

    /// Creates the plugin's data directory and its host-written `downloads/` child, idempotently.
    ///
    /// `downloads/` is created eagerly because the surface layer writes there before the plugin
    /// process has necessarily started; the plugin must never be the one creating it.
    pub fn ensure(&self, plugin_id: &PluginId) -> io::Result<PathBuf> {
        let directory = self.path_for(plugin_id);
        std::fs::create_dir_all(directory.join(DOWNLOADS_DIRECTORY))?;
        Ok(directory)
    }

    /// Removes the plugin's data directory if it exists; a missing directory is not an error.
    pub fn remove(&self, plugin_id: &PluginId) -> io::Result<()> {
        let directory = self.path_for(plugin_id);
        match std::fs::remove_dir_all(&directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Returns the `plugins/data` root shared by every plugin.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Locates `<data-dir>/plugins/logs/<namespace>/<name>/` directories.
///
/// This is a third persistent tree beside installed packages and plugin data: `ora/storage/*`
/// resolves against the data tree, so no logical storage path can reach it, and the log sink
/// creates it level by level rather than eagerly so a foreign path under a plugin's name is
/// refused instead of adopted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLogDirectories {
    root: PathBuf,
}

impl PluginLogDirectories {
    /// Anchors plugin logs below the same data directory that holds packages and plugin data.
    pub fn new(data_directory: impl Into<PathBuf>) -> Self {
        Self {
            root: data_directory
                .into()
                .join(PLUGINS_DIRECTORY)
                .join(LOGS_DIRECTORY),
        }
    }

    /// Returns the plugin's log directory without touching the filesystem.
    pub fn path_for(&self, plugin_id: &PluginId) -> PathBuf {
        self.root.join(plugin_id.namespace()).join(plugin_id.name())
    }

    /// Returns the `plugins/logs` root shared by every plugin.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Removes the plugin's log directory if it exists; a missing directory is not an error.
    pub fn remove(&self, plugin_id: &PluginId) -> io::Result<()> {
        match std::fs::remove_dir_all(self.path_for(plugin_id)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginDataDirectories, PluginLogDirectories};
    use ora_domain::PluginId;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    /// `ensure` creates the namespaced plugin directory plus `downloads/` and is safe to call
    /// repeatedly.
    #[test]
    fn ensure_creates_plugin_and_downloads_directories() {
        let temp_dir = TempDir::new().expect("create data directory");
        let directories = PluginDataDirectories::new(temp_dir.path());
        let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");

        let first = directories
            .ensure(&plugin_id)
            .expect("ensure data directory");
        let second = directories.ensure(&plugin_id).expect("ensure again");

        let expected = temp_dir
            .path()
            .join("plugins")
            .join("data")
            .join("official")
            .join("ora.example");
        assert_eq!(
            (first, second, expected.join("downloads").is_dir()),
            (expected.clone(), expected, true),
        );
    }

    /// Log directories sit beside, never inside, the data tree and are removed as a whole.
    #[test]
    fn log_directories_are_a_sibling_tree_keyed_by_identity() {
        let temp_dir = TempDir::new().expect("create data directory");
        let logs = PluginLogDirectories::new(temp_dir.path());
        let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");
        let directory = logs.path_for(&plugin_id);
        std::fs::create_dir_all(&directory).expect("create log directory");
        std::fs::write(directory.join("plugin.log"), "{}\n").expect("write log");

        logs.remove(&plugin_id).expect("remove log directory");
        logs.remove(&plugin_id).expect("remove missing directory");

        assert_eq!(
            (
                directory.clone(),
                directory.exists(),
                PluginDataDirectories::new(temp_dir.path())
                    .path_for(&plugin_id)
                    .starts_with(logs.root()),
            ),
            (
                temp_dir
                    .path()
                    .join("plugins")
                    .join("logs")
                    .join("official")
                    .join("ora.example"),
                false,
                false,
            )
        );
    }

    /// `remove` deletes everything below the plugin directory and tolerates a missing one.
    #[test]
    fn remove_deletes_the_plugin_directory_and_ignores_missing() {
        let temp_dir = TempDir::new().expect("create data directory");
        let directories = PluginDataDirectories::new(temp_dir.path());
        let plugin_id = PluginId::new("official", "ora.example").expect("plugin id");
        let directory = directories
            .ensure(&plugin_id)
            .expect("ensure data directory");
        std::fs::write(directory.join("downloads").join("a.zip"), b"zip").expect("write file");

        directories
            .remove(&plugin_id)
            .expect("remove data directory");
        directories
            .remove(&plugin_id)
            .expect("remove missing directory");

        assert_eq!(directory.exists(), false);
    }
}
