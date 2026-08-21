use crate::MAX_MANIFEST_BYTES;
use crate::issue::{PluginDiscoveryIssue, PluginDiscoveryIssueKind};
use crate::logo;
use crate::validation::{InstalledPlugin, validate};
use ora_plugin_manifest::{ManifestError, PluginManifest};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::str;

pub(crate) struct PluginDiscovery {
    pub installed_plugins: Vec<InstalledPlugin>,
    pub discovery_issues: Vec<PluginDiscoveryIssue>,
}

/// Discovers valid direct child plugin packages below `<data>/plugins/installed`.
pub(crate) fn discover(data_dir: &Path) -> PluginDiscovery {
    let installed_root = data_dir.join("plugins").join("installed");
    let mut issues = Vec::new();
    let entries = match sorted_package_directories(&installed_root, &mut issues) {
        Some(entries) => entries,
        None => {
            return PluginDiscovery {
                installed_plugins: Vec::new(),
                discovery_issues: issues,
            };
        }
    };

    let mut installed_plugins = Vec::new();
    let mut first_path_by_id = HashMap::<String, PathBuf>::new();
    for package_root in entries {
        let manifest_path = package_root.join("orax.toml");
        // An unusable icon is reported on its own and never blocks the package: presentation
        // metadata must not decide whether a plugin is discovered.
        let logo = match logo::read(&package_root) {
            Ok(logo) => logo,
            Err(issue) => {
                issues.push(issue);
                None
            }
        };
        match read_and_validate_manifest(&package_root, &manifest_path, logo) {
            Ok(plugin) => {
                if let Some(first_path) = first_path_by_id.get(&plugin.id) {
                    issues.push(PluginDiscoveryIssue::new(
                        manifest_path,
                        PluginDiscoveryIssueKind::DuplicatePluginId,
                        Some("id".to_string()),
                        format!(
                            "plugin id `{}` was already discovered at {}",
                            plugin.id,
                            first_path.display()
                        ),
                    ));
                } else {
                    first_path_by_id.insert(plugin.id.clone(), plugin.package_root.clone());
                    installed_plugins.push(plugin);
                }
            }
            Err(issue) => issues.push(issue),
        }
    }

    installed_plugins.sort_by(|left, right| left.id.cmp(&right.id));
    PluginDiscovery {
        installed_plugins,
        discovery_issues: issues,
    }
}

/// Returns real direct child directories in reproducible path order.
fn sorted_package_directories(
    installed_root: &Path,
    issues: &mut Vec<PluginDiscoveryIssue>,
) -> Option<Vec<PathBuf>> {
    let read_dir = match fs::read_dir(installed_root) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            issues.push(PluginDiscoveryIssue::new(
                installed_root.to_path_buf(),
                PluginDiscoveryIssueKind::RootUnreadable,
                None,
                error.to_string(),
            ));
            return None;
        }
    };

    let mut directories = Vec::new();
    for entry in read_dir {
        match entry {
            Ok(entry) => match entry.file_type() {
                Ok(file_type) if file_type.is_dir() => directories.push(entry.path()),
                Ok(_) => {}
                Err(error) => issues.push(PluginDiscoveryIssue::new(
                    entry.path(),
                    PluginDiscoveryIssueKind::EntryUnreadable,
                    None,
                    error.to_string(),
                )),
            },
            Err(error) => issues.push(PluginDiscoveryIssue::new(
                installed_root.to_path_buf(),
                PluginDiscoveryIssueKind::EntryUnreadable,
                None,
                error.to_string(),
            )),
        }
    }
    directories.sort();

    Some(directories)
}

/// Reads one bounded TOML manifest, parses it with the shared manifest crate, and validates the fixed entrypoint.
fn read_and_validate_manifest(
    package_root: &Path,
    manifest_path: &Path,
    logo: Option<String>,
) -> Result<InstalledPlugin, PluginDiscoveryIssue> {
    let file_type = match fs::symlink_metadata(manifest_path) {
        Ok(metadata) => metadata.file_type(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(PluginDiscoveryIssue::new(
                manifest_path.to_path_buf(),
                PluginDiscoveryIssueKind::MissingManifest,
                None,
                "plugin directory does not contain orax.toml",
            ));
        }
        Err(error) => {
            return Err(PluginDiscoveryIssue::new(
                manifest_path.to_path_buf(),
                PluginDiscoveryIssueKind::ManifestUnreadable,
                None,
                error.to_string(),
            ));
        }
    };
    if !file_type.is_file() {
        return Err(PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::ManifestNotFile,
            None,
            "orax.toml must be a regular file",
        ));
    }

    let bytes = read_bounded(manifest_path)?;
    let source = str::from_utf8(&bytes).map_err(|error| {
        PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidToml,
            None,
            format!("orax.toml is not valid UTF-8: {error}"),
        )
    })?;
    let manifest = PluginManifest::parse_installed(source).map_err(|error| match error {
        ManifestError::InvalidToml { source, .. } => PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidToml,
            None,
            source.to_string(),
        ),
        ManifestError::UnsupportedResolver { found } => PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidManifest,
            Some("resolver".to_string()),
            format!("unsupported plugin manifest resolver {found}"),
        ),
        ManifestError::InvalidField { field, reason } => PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidManifest,
            Some(field.to_string()),
            reason.to_string(),
        ),
    })?;

    validate(package_root, &manifest, logo).map_err(|error| {
        PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidManifest,
            Some(error.field_path().to_string()),
            error.to_string(),
        )
    })
}

/// Reads at most one byte beyond the supported manifest size to detect concurrent growth.
fn read_bounded(manifest_path: &Path) -> Result<Vec<u8>, PluginDiscoveryIssue> {
    let file = File::open(manifest_path).map_err(|error| {
        PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::ManifestUnreadable,
            None,
            error.to_string(),
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            PluginDiscoveryIssue::new(
                manifest_path.to_path_buf(),
                PluginDiscoveryIssueKind::ManifestUnreadable,
                None,
                error.to_string(),
            )
        })?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::ManifestTooLarge,
            None,
            format!("manifest exceeds the {MAX_MANIFEST_BYTES}-byte limit"),
        ));
    }

    Ok(bytes)
}
