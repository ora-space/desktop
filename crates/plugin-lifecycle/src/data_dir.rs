//! Owns the per-plugin writable data directory below the Ora data root.
//!
//! Installed packages are read-only; this is the only place a plugin process may write. The
//! directory name is the plugin id itself, which manifest validation already bounds to slug
//! segments, so it is safe on every platform without further escaping.

use ora_domain::PluginId;
use std::io;
use std::path::{Path, PathBuf};

const PLUGIN_DATA_ROOT: &str = "plugin-data";
const DOWNLOADS_DIRECTORY: &str = "downloads";

/// Creates and locates `<data-dir>/plugin-data/<plugin_id>/` directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDataDirectories {
    root: PathBuf,
}

impl PluginDataDirectories {
    /// Anchors plugin data below the same data directory that holds installed packages.
    pub fn new(data_directory: impl Into<PathBuf>) -> Self {
        Self {
            root: data_directory.into().join(PLUGIN_DATA_ROOT),
        }
    }

    /// Returns the plugin's data directory without touching the filesystem.
    pub fn path_for(&self, plugin_id: &PluginId) -> PathBuf {
        self.root.join(plugin_id.to_string())
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

    /// Returns the `plugin-data` root shared by every plugin.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::PluginDataDirectories;
    use ora_domain::PluginId;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    /// `ensure` creates the plugin directory plus `downloads/` and is safe to call repeatedly.
    #[test]
    fn ensure_creates_plugin_and_downloads_directories() {
        let temp_dir = TempDir::new().expect("create data directory");
        let directories = PluginDataDirectories::new(temp_dir.path());
        let plugin_id = PluginId::new("ora.example");

        let first = directories
            .ensure(&plugin_id)
            .expect("ensure data directory");
        let second = directories.ensure(&plugin_id).expect("ensure again");

        let expected = temp_dir.path().join("plugin-data").join("ora.example");
        assert_eq!(
            (first, second, expected.join("downloads").is_dir()),
            (expected.clone(), expected, true),
        );
    }

    /// `remove` deletes everything below the plugin directory and tolerates a missing one.
    #[test]
    fn remove_deletes_the_plugin_directory_and_ignores_missing() {
        let temp_dir = TempDir::new().expect("create data directory");
        let directories = PluginDataDirectories::new(temp_dir.path());
        let plugin_id = PluginId::new("ora.example");
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
