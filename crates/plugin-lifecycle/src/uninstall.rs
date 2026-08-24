use super::PluginLifecycleError;
use ora_contracts::PluginDataDisposition;
use ora_plugin_manager::InstalledPlugin as DiscoveredPlugin;
use std::path::{Path, PathBuf};

/// Holds same-volume moves until uninstall's package and data decision has committed.
pub(crate) struct StagedUninstall {
    staging_root: PathBuf,
    moved: Vec<(PathBuf, PathBuf)>,
}

impl StagedUninstall {
    /// Restores every successful move in reverse order after a later staging or repository failure.
    pub(crate) fn rollback(self) -> Result<(), PluginLifecycleError> {
        for (original, staged) in self.moved.into_iter().rev() {
            if staged.exists() {
                std::fs::rename(&staged, &original).map_err(|source| {
                    PluginLifecycleError::PackageRemoval {
                        path: staged,
                        source,
                    }
                })?;
            }
        }
        let _ = std::fs::remove_dir_all(&self.staging_root);
        Ok(())
    }

    /// Removes committed staging content; callers may retry independently after a failure.
    pub(crate) fn cleanup(self) -> std::io::Result<()> {
        std::fs::remove_dir_all(self.staging_root)
    }
}

/// Stages code and, when selected, plugin-global data through atomic same-volume moves.
pub(crate) fn stage_uninstall(
    data_directory: &Path,
    plugin: &DiscoveredPlugin,
    data_disposition: PluginDataDisposition,
) -> Result<StagedUninstall, PluginLifecycleError> {
    let package_name_root =
        plugin
            .package_root
            .parent()
            .ok_or_else(|| PluginLifecycleError::PackageRemoval {
                path: plugin.package_root.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "installed package root does not contain a version directory",
                ),
            })?;
    if !package_name_root.is_dir() {
        return Err(PluginLifecycleError::PackageRemoval {
            path: package_name_root.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "installed package name root is not a directory",
            ),
        });
    }
    let staging_parent = data_directory.join(".uninstall-staging");
    std::fs::create_dir_all(&staging_parent).map_err(|source| {
        PluginLifecycleError::PackageRemoval {
            path: staging_parent.clone(),
            source,
        }
    })?;
    let mut staging_root = None;
    for attempt in 0_u16..=u16::MAX {
        let candidate = staging_parent.join(format!("{}-{attempt}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => {
                staging_root = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(PluginLifecycleError::PackageRemoval {
                    path: candidate,
                    source,
                });
            }
        }
    }
    let staging_root = staging_root.ok_or_else(|| PluginLifecycleError::PackageRemoval {
        path: staging_parent,
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate uninstall staging directory",
        ),
    })?;
    let mut staged = StagedUninstall {
        staging_root: staging_root.clone(),
        moved: Vec::new(),
    };
    let staged_installation = staging_root.join("installation");
    if let Err(source) = std::fs::rename(package_name_root, &staged_installation) {
        let _ = std::fs::remove_dir_all(&staging_root);
        return Err(PluginLifecycleError::PackageRemoval {
            path: package_name_root.to_path_buf(),
            source,
        });
    }
    staged
        .moved
        .push((package_name_root.to_path_buf(), staged_installation));

    if matches!(data_disposition, PluginDataDisposition::Delete) {
        let data_root = plugin_data_root(data_directory, &plugin.id)?;
        if data_root.exists() {
            let staged_data = staging_root.join("data");
            if let Err(source) = std::fs::rename(&data_root, &staged_data) {
                staged.rollback()?;
                return Err(PluginLifecycleError::PackageRemoval {
                    path: data_root,
                    source,
                });
            }
            staged.moved.push((data_root, staged_data));
        }
    }
    Ok(staged)
}

/// Resolves the host-owned data directory for a namespaced discovered plugin identifier.
pub(crate) fn plugin_data_root(
    data_directory: &Path,
    plugin_id: &str,
) -> Result<PathBuf, PluginLifecycleError> {
    let Some((namespace, name)) = plugin_id.split_once('/') else {
        return Err(PluginLifecycleError::PackageRemoval {
            path: data_directory.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "plugin identifier is not namespaced",
            ),
        });
    };
    Ok(data_directory.join("data").join(namespace).join(name))
}
