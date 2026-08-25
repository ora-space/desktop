use crate::MAX_MANIFEST_BYTES;
use crate::issue::{PluginDiscoveryIssue, PluginDiscoveryIssueKind};
use crate::logo;
use crate::validation::{InstalledPlugin, validate};
use ora_domain::PluginId;
use ora_plugin_manifest::{ManifestError, PluginManifest};
use semver::Version;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Directory levels below the Ora data directory that hold installed packages.
const PLUGINS_DIRECTORY: &str = "plugins";
const INSTALLED_DIRECTORY: &str = "installed";
/// File name of the package manifest inside one installed package directory.
pub const MANIFEST_FILE_NAME: &str = "orax.toml";

pub(crate) struct PluginDiscovery {
    pub installed_plugins: Vec<InstalledPlugin>,
    pub discovery_issues: Vec<PluginDiscoveryIssue>,
}

/// Returns `<data-dir>/plugins/installed`, the root every installed package lives below.
pub fn installed_root(data_dir: &Path) -> PathBuf {
    data_dir.join(PLUGINS_DIRECTORY).join(INSTALLED_DIRECTORY)
}

/// Discovers the highest installed version of every `<namespace>/<name>` package below the
/// installed root and isolates every recoverable failure so one broken package never hides its
/// siblings.
///
/// The directory names are not part of a package's identity: the manifest alone names the plugin,
/// and two packages claiming the same id are reported rather than silently merged. The version
/// directory, however, must agree with the manifest so an installation can never advertise one
/// version while running another.
pub(crate) fn discover(data_dir: &Path) -> PluginDiscovery {
    let installed_root = installed_root(data_dir);
    let mut issues = Vec::new();
    let Some(package_roots) = sorted_package_directories(&installed_root, &mut issues) else {
        return PluginDiscovery {
            installed_plugins: Vec::new(),
            discovery_issues: issues,
        };
    };

    let mut installed_plugins = Vec::new();
    let mut first_root_by_id = HashMap::<PluginId, PathBuf>::new();
    for (package_root, directory_version) in package_roots {
        let manifest_path = package_root.join(MANIFEST_FILE_NAME);
        // An unusable icon is reported on its own and never blocks the package: presentation
        // metadata must not decide whether a plugin is discovered.
        let logo = match logo::read(&package_root) {
            Ok(logo) => logo,
            Err(issue) => {
                issues.push(issue);
                None
            }
        };
        match read_and_validate_manifest(&package_root, &manifest_path, logo, &directory_version) {
            Ok(plugin) => {
                if let Some(first_root) = first_root_by_id.get(&plugin.id) {
                    issues.push(PluginDiscoveryIssue::new(
                        manifest_path,
                        PluginDiscoveryIssueKind::DuplicatePluginId,
                        Some("identifier".to_string()),
                        format!(
                            "plugin `{}` was already discovered at {}",
                            plugin.id,
                            first_root.display()
                        ),
                    ));
                } else {
                    first_root_by_id.insert(plugin.id.clone(), plugin.package_root.clone());
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

/// Selects the highest semantic-version directory for every namespace and package name, in
/// reproducible path order, or `None` when the installed root cannot be listed at all. A missing
/// root is an empty installation.
fn sorted_package_directories(
    installed_root: &Path,
    issues: &mut Vec<PluginDiscoveryIssue>,
) -> Option<Vec<(PathBuf, Version)>> {
    let namespaces = sorted_directories(installed_root, PluginRoot::Installed, issues)?;
    let mut selected = Vec::new();
    for namespace_root in namespaces {
        let Some(package_names) = sorted_directories(&namespace_root, PluginRoot::Nested, issues)
        else {
            continue;
        };
        for package_name_root in package_names {
            let Some(version_roots) =
                sorted_directories(&package_name_root, PluginRoot::Nested, issues)
            else {
                continue;
            };
            let mut versions = Vec::new();
            for version_root in version_roots {
                let Some(value) = version_root.file_name().and_then(|value| value.to_str()) else {
                    issues.push(PluginDiscoveryIssue::new(
                        version_root,
                        PluginDiscoveryIssueKind::InvalidInstallPath,
                        None,
                        "plugin version directory name must be valid UTF-8",
                    ));
                    continue;
                };
                match Version::parse(value) {
                    Ok(version) => versions.push((version_root, version)),
                    Err(error) => issues.push(PluginDiscoveryIssue::new(
                        version_root,
                        PluginDiscoveryIssueKind::InvalidInstallPath,
                        None,
                        format!("plugin version directory is not valid SemVer: {error}"),
                    )),
                }
            }
            // Selecting before reading the manifest prevents a corrupt new installation from
            // silently reactivating an older version the user no longer intended to run.
            versions.sort_by(|(left_path, left_version), (right_path, right_version)| {
                left_version
                    .cmp(right_version)
                    .then_with(|| left_path.cmp(right_path))
            });
            if let Some(highest) = versions.pop() {
                selected.push(highest);
            }
        }
    }

    Some(selected)
}

/// Distinguishes a missing top-level installation root from broken nested directories.
#[derive(Clone, Copy)]
enum PluginRoot {
    Installed,
    Nested,
}

/// Returns real child directories in reproducible path order without following symlinks.
fn sorted_directories(
    root: &Path,
    root_kind: PluginRoot,
    issues: &mut Vec<PluginDiscoveryIssue>,
) -> Option<Vec<PathBuf>> {
    let read_dir = match fs::read_dir(root) {
        Ok(read_dir) => read_dir,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && matches!(root_kind, PluginRoot::Installed) =>
        {
            return None;
        }
        Err(error) => {
            issues.push(PluginDiscoveryIssue::new(
                root.to_path_buf(),
                match root_kind {
                    PluginRoot::Installed => PluginDiscoveryIssueKind::RootUnreadable,
                    PluginRoot::Nested => PluginDiscoveryIssueKind::EntryUnreadable,
                },
                None,
                error.to_string(),
            ));
            return None;
        }
    };

    let mut directories = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                issues.push(PluginDiscoveryIssue::new(
                    root.to_path_buf(),
                    PluginDiscoveryIssueKind::EntryUnreadable,
                    None,
                    error.to_string(),
                ));
                continue;
            }
        };
        // `DirEntry::file_type` never follows symlinks, so a linked directory is skipped: the
        // installer only ever writes real directories and a link could point anywhere.
        match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => directories.push(entry.path()),
            Ok(_) => {}
            Err(error) => issues.push(PluginDiscoveryIssue::new(
                entry.path(),
                PluginDiscoveryIssueKind::EntryUnreadable,
                None,
                error.to_string(),
            )),
        }
    }
    directories.sort();

    Some(directories)
}

/// Reads one bounded manifest, parses it with the shared manifest crate, and applies the
/// host-side checks.
fn read_and_validate_manifest(
    package_root: &Path,
    manifest_path: &Path,
    logo: Option<String>,
    directory_version: &Version,
) -> Result<InstalledPlugin, PluginDiscoveryIssue> {
    let file_type = match fs::symlink_metadata(manifest_path) {
        Ok(metadata) => metadata.file_type(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(PluginDiscoveryIssue::new(
                manifest_path.to_path_buf(),
                PluginDiscoveryIssueKind::MissingManifest,
                None,
                format!("plugin directory does not contain {MANIFEST_FILE_NAME}"),
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
            format!("{MANIFEST_FILE_NAME} must be a regular file"),
        ));
    }

    let bytes = read_bounded(manifest_path)?;
    let source = std::str::from_utf8(&bytes).map_err(|error| {
        PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidToml,
            None,
            format!("{MANIFEST_FILE_NAME} is not valid UTF-8: {error}"),
        )
    })?;
    let manifest = PluginManifest::parse_installed(source).map_err(|error| match error {
        ManifestError::InvalidToml { source, path, .. } => PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidToml,
            path,
            source.message().to_string(),
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

    let plugin = validate(package_root, &manifest, logo).map_err(|error| {
        PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidManifest,
            Some(error.field_path().to_string()),
            error.to_string(),
        )
    })?;
    if plugin.version != *directory_version {
        return Err(PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidManifest,
            Some("version".to_string()),
            format!(
                "package version {} does not match installation directory {directory_version}",
                plugin.version
            ),
        ));
    }

    Ok(plugin)
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
