use super::PluginLifecycleError;
use crate::{PluginDataDirectories, PluginLogDirectories};
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
        // The data tree and the sibling log tree are two halves of one decision: either both
        // move into staging or neither does. The log directory is only movable because the
        // caller has already stopped the process and waited for its log writer to release the
        // file; on Windows an open handle would make this rename fail, which then rolls back
        // the package and data moves rather than leaving a half-deleted plugin.
        let owned_trees = [
            ("data", plugin_data_root(data_directory, &plugin.id)),
            ("logs", plugin_log_root(data_directory, &plugin.id)),
        ];
        for (label, tree_root) in owned_trees {
            if !file_system.exists(&tree_root) {
                continue;
            }
            let staged_tree = staging_root.join(label);
            if let Err(source) = file_system.rename(&tree_root, &staged_tree) {
                if let Err(rollback_error) = staged.rollback() {
                    ora_warn!(
                        tree_root = %tree_root.display(),
                        %source,
                        %rollback_error,
                        "could not stage a plugin-owned tree and rollback also failed"
                    );
                    return Err(rollback_error);
                }
                return Err(PluginLifecycleError::UninstallStaging {
                    path: tree_root,
                    source,
                });
            }
            staged.moved.push((tree_root, staged_tree));
        }
    }
    Ok(staged)
}

/// Resolves the host-owned data directory for one plugin identity.
pub(crate) fn plugin_data_root(data_directory: &Path, plugin_id: &PluginId) -> PathBuf {
    PluginDataDirectories::new(data_directory).path_for(plugin_id)
}

/// Resolves the host-managed log directory for one plugin identity.
pub(crate) fn plugin_log_root(data_directory: &Path, plugin_id: &PluginId) -> PathBuf {
    PluginLogDirectories::new(data_directory).path_for(plugin_id)
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

    /// Fails the N-th rename (0-based) so a test can break staging after specific moves.
    #[derive(Clone)]
    struct FailNthRename {
        calls: Arc<AtomicUsize>,
        failing_call: usize,
    }

    impl UninstallFileSystem for FailNthRename {
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
            if self.calls.fetch_add(/*val*/ 1, Ordering::SeqCst) == self.failing_call {
                return Err(std::io::Error::other("injected move failure"));
            }
            fs::rename(source, destination)
        }

        fn remove_dir_all(&self, path: &Path) -> std::io::Result<()> {
            fs::remove_dir_all(path)
        }
    }

    /// Writes one installed package plus its data and log trees, returning the discovered plugin
    /// and the three roots.
    fn installed_with_data_and_logs(
        temporary: &TempDir,
    ) -> (
        ora_plugin_manager::InstalledPlugin,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let package_root = temporary
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join("example")
            .join("1.0.0");
        fs::create_dir_all(&package_root).expect("create package root");
        fs::write(
            package_root.join("main.js"),
            "export {};
",
        )
        .expect("write entrypoint");
        fs::write(
            package_root.join("orax.toml"),
            "resolver = 1
identifier = \"example\"
namespace = \"official\"
kind = \"agent\"
version = \"1.0.0\"
description = \"Example\"
",
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
        let log_root = temporary
            .path()
            .join("plugins")
            .join("logs")
            .join("official")
            .join("example");
        fs::create_dir_all(&log_root).expect("create plugin logs");
        fs::write(
            log_root.join("plugin.log"),
            "{}
",
        )
        .expect("write plugin log");
        let plugin = PluginManager::discover(temporary.path())
            .installed_plugins()
            .first()
            .cloned()
            .expect("discover plugin");
        (plugin, package_root, data_root, log_root)
    }

    /// Deleting data stages the package, the data tree, and the log tree together; committing
    /// removes all three and retaining data leaves both trees alone.
    #[test]
    fn stages_the_log_tree_with_the_data_tree() {
        for (disposition, expect_trees) in [
            (PluginDataDisposition::Delete, false),
            (PluginDataDisposition::Retain, true),
        ] {
            let temporary = TempDir::new().expect("create uninstall root");
            let (plugin, package_root, data_root, log_root) =
                installed_with_data_and_logs(&temporary);

            let staged = stage_uninstall_with_file_system(
                temporary.path(),
                &plugin,
                disposition,
                FailNthRename {
                    calls: Arc::new(AtomicUsize::new(0)),
                    failing_call: usize::MAX,
                },
            )
            .expect("staging succeeds");
            staged.cleanup().expect("cleanup staging");

            assert_eq!(
                (
                    package_root.exists(),
                    data_root.join("store.json").is_file(),
                    log_root.join("plugin.log").is_file(),
                ),
                (false, expect_trees, expect_trees),
                "{disposition:?}"
            );
        }
    }

    /// A log-tree move failure rolls the already staged installation and data back to their
    /// exact source paths, so a delete-data uninstall never reports success with logs left over.
    #[test]
    fn rolls_back_installation_and_data_when_staging_logs_fails() {
        let temporary = TempDir::new().expect("create uninstall root");
        let (plugin, package_root, data_root, log_root) = installed_with_data_and_logs(&temporary);

        let error = stage_uninstall_with_file_system(
            temporary.path(),
            &plugin,
            PluginDataDisposition::Delete,
            FailNthRename {
                calls: Arc::new(AtomicUsize::new(0)),
                failing_call: 2,
            },
        )
        .err()
        .expect("staging must fail");

        assert_eq!(
            (
                error.to_string(),
                package_root.join("main.js").is_file(),
                data_root.join("store.json").is_file(),
                log_root.join("plugin.log").is_file(),
            ),
            (
                format!(
                    "failed to stage plugin uninstall at `{}`",
                    log_root.display()
                ),
                true,
                true,
                true,
            )
        );
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
        let file_system = FailNthRename {
            calls: Arc::new(AtomicUsize::new(0)),
            failing_call: 1,
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
