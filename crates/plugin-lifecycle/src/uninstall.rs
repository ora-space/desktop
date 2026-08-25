use super::PluginLifecycleError;
use crate::PluginDataDirectories;
use ora_contracts::PluginDataDisposition;
use ora_domain::PluginId;
use ora_logging::ora_warn;
use ora_plugin_manager::InstalledPlugin as DiscoveredPlugin;
use std::path::{Path, PathBuf};

/// Supplies uninstall's safety-critical filesystem transitions through a statically dispatched seam.
///
/// Implementations must keep `rename` on one volume and report destination collisions rather than
/// replacing unrelated data. Tests use this port to force failures after individual staged moves.
pub(crate) trait UninstallFileSystem: Clone {
    /// Reports whether one path currently names a directory.
    fn is_directory(&self, path: &Path) -> bool;
    /// Reports whether one path currently exists.
    fn exists(&self, path: &Path) -> bool;
    /// Creates a complete directory hierarchy.
    fn create_dir_all(&self, path: &Path) -> std::io::Result<()>;
    /// Creates exactly one directory and preserves AlreadyExists.
    fn create_dir(&self, path: &Path) -> std::io::Result<()>;
    /// Atomically moves one path on its current volume.
    fn rename(&self, source: &Path, destination: &Path) -> std::io::Result<()>;
    /// Removes one directory tree after uninstall has committed.
    fn remove_dir_all(&self, path: &Path) -> std::io::Result<()>;
}

/// Production adapter for same-volume uninstall staging operations.
#[derive(Clone, Copy)]
pub(crate) struct StandardUninstallFileSystem;

impl UninstallFileSystem for StandardUninstallFileSystem {
    fn is_directory(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn create_dir(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir(path)
    }

    fn rename(&self, source: &Path, destination: &Path) -> std::io::Result<()> {
        std::fs::rename(source, destination)
    }

    fn remove_dir_all(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_dir_all(path)
    }
}

/// Holds same-volume moves until uninstall's package and data decision has committed.
pub(crate) struct StagedUninstall<FileSystem = StandardUninstallFileSystem> {
    staging_root: PathBuf,
    moved: Vec<(PathBuf, PathBuf)>,
    file_system: FileSystem,
}

impl<FileSystem> StagedUninstall<FileSystem>
where
    FileSystem: UninstallFileSystem,
{
    /// Restores every successful move in reverse order after a later staging or repository failure.
    pub(crate) fn rollback(self) -> Result<(), PluginLifecycleError> {
        let mut failure = None;
        for (original, staged) in self.moved.into_iter().rev() {
            if self.file_system.exists(&staged)
                && let Err(source) = self.file_system.rename(&staged, &original)
            {
                ora_warn!(
                    staged = %staged.display(),
                    original = %original.display(),
                    %source,
                    "could not restore one staged plugin uninstall path"
                );
                failure.get_or_insert(PluginLifecycleError::UninstallStaging {
                    path: staged,
                    source,
                });
            }
        }
        if let Err(source) = self.file_system.remove_dir_all(&self.staging_root) {
            ora_warn!(
                staging_root = %self.staging_root.display(),
                %source,
                "could not remove plugin uninstall staging directory after rollback"
            );
            failure.get_or_insert(PluginLifecycleError::UninstallStaging {
                path: self.staging_root,
                source,
            });
        }
        failure.map_or(Ok(()), Err)
    }

    /// Removes committed staging content; callers may retry independently after a failure.
    pub(crate) fn cleanup(&self) -> std::io::Result<()> {
        self.file_system.remove_dir_all(&self.staging_root)
    }
}

/// Stages code and, when selected, plugin-global data through atomic same-volume moves.
pub(crate) fn stage_uninstall(
    data_directory: &Path,
    plugin: &DiscoveredPlugin,
    data_disposition: PluginDataDisposition,
) -> Result<StagedUninstall, PluginLifecycleError> {
    stage_uninstall_with_file_system(
        data_directory,
        plugin,
        data_disposition,
        StandardUninstallFileSystem,
    )
}

/// Implements staged uninstall against an injected filesystem for deterministic rollback tests.
fn stage_uninstall_with_file_system<FileSystem>(
    data_directory: &Path,
    plugin: &DiscoveredPlugin,
    data_disposition: PluginDataDisposition,
    file_system: FileSystem,
) -> Result<StagedUninstall<FileSystem>, PluginLifecycleError>
where
    FileSystem: UninstallFileSystem,
{
    let package_name_root =
        plugin
            .package_root
            .parent()
            .ok_or_else(|| PluginLifecycleError::UninstallStaging {
                path: plugin.package_root.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "installed package root does not contain a version directory",
                ),
            })?;
    if !file_system.is_directory(package_name_root) {
        return Err(PluginLifecycleError::UninstallStaging {
            path: package_name_root.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "installed package name root is not a directory",
            ),
        });
    }
    let staging_parent = data_directory.join(".uninstall-staging");
    file_system
        .create_dir_all(&staging_parent)
        .map_err(|source| PluginLifecycleError::UninstallStaging {
            path: staging_parent.clone(),
            source,
        })?;
    let mut staging_root = None;
    for attempt in 0_u16..=u16::MAX {
        let candidate = staging_parent.join(format!("{}-{attempt}", std::process::id()));
        match file_system.create_dir(&candidate) {
            Ok(()) => {
                staging_root = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(PluginLifecycleError::UninstallStaging {
                    path: candidate,
                    source,
                });
            }
        }
    }
    let staging_root = staging_root.ok_or_else(|| PluginLifecycleError::UninstallStaging {
        path: staging_parent,
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate uninstall staging directory",
        ),
    })?;
    let mut staged = StagedUninstall {
        staging_root: staging_root.clone(),
        moved: Vec::new(),
        file_system: file_system.clone(),
    };
    let staged_installation = staging_root.join("installation");
    if let Err(source) = file_system.rename(package_name_root, &staged_installation) {
        if let Err(cleanup_error) = file_system.remove_dir_all(&staging_root) {
            ora_warn!(
                staging_root = %staging_root.display(),
                %source,
                %cleanup_error,
                "could not stage plugin installation and staging directory cleanup also failed"
            );
        }
        return Err(PluginLifecycleError::UninstallStaging {
            path: package_name_root.to_path_buf(),
            source,
        });
    }
    staged
        .moved
        .push((package_name_root.to_path_buf(), staged_installation));

    if matches!(data_disposition, PluginDataDisposition::Delete) {
        let data_root = plugin_data_root(data_directory, &plugin.id);
        if file_system.exists(&data_root) {
            let staged_data = staging_root.join("data");
            if let Err(source) = file_system.rename(&data_root, &staged_data) {
                if let Err(rollback_error) = staged.rollback() {
                    ora_warn!(
                        data_root = %data_root.display(),
                        %source,
                        %rollback_error,
                        "could not stage plugin data and rollback also failed"
                    );
                    return Err(rollback_error);
                }
                return Err(PluginLifecycleError::UninstallStaging {
                    path: data_root,
                    source,
                });
            }
            staged.moved.push((data_root, staged_data));
        }
    }
    Ok(staged)
}

/// Resolves the host-owned data directory for one plugin identity.
pub(crate) fn plugin_data_root(data_directory: &Path, plugin_id: &PluginId) -> PathBuf {
    PluginDataDirectories::new(data_directory).path_for(plugin_id)
}

#[cfg(test)]
mod tests {
    use super::{UninstallFileSystem, stage_uninstall_with_file_system};
    use ora_contracts::PluginDataDisposition;
    use ora_plugin_manager::PluginManager;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    #[derive(Clone)]
    struct FailSecondRename {
        calls: Arc<AtomicUsize>,
    }

    impl UninstallFileSystem for FailSecondRename {
        fn is_directory(&self, path: &Path) -> bool {
            path.is_dir()
        }

        fn exists(&self, path: &Path) -> bool {
            path.exists()
        }

        fn create_dir_all(&self, path: &Path) -> std::io::Result<()> {
            fs::create_dir_all(path)
        }

        fn create_dir(&self, path: &Path) -> std::io::Result<()> {
            fs::create_dir(path)
        }

        fn rename(&self, source: &Path, destination: &Path) -> std::io::Result<()> {
            if self.calls.fetch_add(/*val*/ 1, Ordering::SeqCst) == 1 {
                return Err(std::io::Error::other("injected data move failure"));
            }
            fs::rename(source, destination)
        }

        fn remove_dir_all(&self, path: &Path) -> std::io::Result<()> {
            fs::remove_dir_all(path)
        }
    }

    /// A data-move failure rolls the already staged installation back to its exact source path.
    #[test]
    fn rolls_back_installation_when_staging_data_fails() {
        let temporary = TempDir::new().expect("create uninstall root");
        let package_root = temporary
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join("example")
            .join("1.0.0");
        fs::create_dir_all(&package_root).expect("create package root");
        fs::write(package_root.join("main.js"), "export {};\n").expect("write entrypoint");
        fs::write(
            package_root.join("orax.toml"),
            "resolver = 1\nidentifier = \"example\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Example\"\n",
        )
        .expect("write manifest");
        let data_root = temporary
            .path()
            .join("plugins")
            .join("data")
            .join("official")
            .join("example");
        fs::create_dir_all(&data_root).expect("create plugin data");
        fs::write(data_root.join("store.json"), "{}").expect("write plugin data");
        let plugin = PluginManager::discover(temporary.path())
            .installed_plugins()
            .first()
            .cloned()
            .expect("discover plugin");
        let file_system = FailSecondRename {
            calls: Arc::new(AtomicUsize::new(0)),
        };

        let error = stage_uninstall_with_file_system(
            temporary.path(),
            &plugin,
            PluginDataDisposition::Delete,
            file_system,
        )
        .err()
        .expect("staging must fail");

        assert_eq!(
            error.to_string(),
            format!(
                "failed to stage plugin uninstall at `{}`",
                data_root.display()
            )
        );
        assert!(package_root.join("main.js").is_file());
        assert!(data_root.join("store.json").is_file());
    }
}
