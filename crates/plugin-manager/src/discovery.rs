use crate::MAX_MANIFEST_BYTES;
use crate::issue::{PluginDiscoveryIssue, PluginDiscoveryIssueKind};
use crate::logo;
use crate::validation::{InstalledPlugin, validate};
use ora_domain::PluginNamespace;
use ora_plugin_manifest::{ManifestError, PluginManifest};
use semver::Version;
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
/// The installed tree is the authority for a package's namespace. A manifest is written by the
/// plugin's author, who cannot know which marketplace source a user installed the package
/// through, so the namespace is only ever recorded by the host — as the first directory level the
/// installer wrote — and is read back from there, not inferred from anything inside the package.
///
/// The remaining identity, the name and version, does come from the manifest, and each is checked
/// against the directory that holds it: a package that disagrees with its own location is
/// reported as corrupt rather than being attributed to whichever of the two the host happened to
/// read first.
pub(crate) fn discover(data_dir: &Path) -> PluginDiscovery {
    let installed_root = installed_root(data_dir);
    let mut issues = Vec::new();
    let Some(package_roots) = sorted_package_directories(&installed_root, &mut issues) else {
        return PluginDiscovery {
            installed_plugins: Vec::new(),
            discovery_issues: issues,
        };
    };

    // Two packages can no longer claim one id: both id segments are directory levels, and the
    // walk visits each `<namespace>/<name>` pair once, so a second copy of a plugin would have to
    // be the same directory. A copy placed under a different name directory is a different id and
    // is rejected earlier, by the manifest-versus-directory check.
    let mut installed_plugins = Vec::new();
    for location in package_roots {
        let manifest_path = location.package_root.join(MANIFEST_FILE_NAME);
        // An unusable icon is reported on its own and never blocks the package: presentation
        // metadata must not decide whether a plugin is discovered.
        let logo = match logo::read(&location.package_root) {
            Ok(logo) => logo,
            Err(issue) => {
                issues.push(issue);
                None
            }
        };
        match read_and_validate_manifest(&location, &manifest_path, logo) {
            Ok(plugin) => installed_plugins.push(plugin),
            Err(issue) => issues.push(issue),
        }
    }

    installed_plugins.sort_by(|left, right| left.id.cmp(&right.id));
    PluginDiscovery {
        installed_plugins,
        discovery_issues: issues,
    }
}

/// Names one installed package directory together with the identity its location asserts.
///
/// The namespace is the whole reason this type exists: it lives nowhere but the path, so it has
/// to be carried from the directory walk to manifest validation instead of being re-derived.
pub(crate) struct InstalledPackageLocation {
    pub package_root: PathBuf,
    pub namespace: PluginNamespace,
    pub directory_name: String,
    pub directory_version: Version,
}

/// Selects the highest semantic-version directory for every namespace and package name, in
/// reproducible path order, or `None` when the installed root cannot be listed at all. A missing
/// root is an empty installation.
///
/// A namespace or name directory the host could not have written — non-UTF-8, or outside the id
/// grammar — is reported and skipped rather than repaired: the host is the only writer of this
/// tree, so such a directory is either corruption or something placed there by hand, and either
/// way guessing an identity for it would be inventing provenance.
fn sorted_package_directories(
    installed_root: &Path,
    issues: &mut Vec<PluginDiscoveryIssue>,
) -> Option<Vec<InstalledPackageLocation>> {
    let namespace_roots = sorted_directories(installed_root, PluginRoot::Installed, issues)?;
    let mut selected = Vec::new();
    for namespace_root in namespace_roots {
        let Some(namespace) = directory_segment(&namespace_root, "namespace", issues) else {
            continue;
        };
        let namespace = match PluginNamespace::parse(&namespace) {
            Ok(namespace) => namespace,
            Err(error) => {
                issues.push(PluginDiscoveryIssue::new(
                    namespace_root,
                    PluginDiscoveryIssueKind::InvalidInstallPath,
                    None,
                    format!("plugin namespace directory is not a usable namespace: {error}"),
                ));
                continue;
            }
        };
        let Some(package_names) = sorted_directories(&namespace_root, PluginRoot::Nested, issues)
        else {
            continue;
        };
        for package_name_root in package_names {
            let Some(directory_name) = directory_segment(&package_name_root, "name", issues) else {
                continue;
            };
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
            if let Some((package_root, directory_version)) = versions.pop() {
                selected.push(InstalledPackageLocation {
                    package_root,
                    namespace: namespace.clone(),
                    directory_name: directory_name.clone(),
                    directory_version,
                });
            }
        }
    }

    Some(selected)
}

/// Reads one directory level's name as UTF-8, reporting a name the host could not have written.
fn directory_segment(
    root: &Path,
    level: &str,
    issues: &mut Vec<PluginDiscoveryIssue>,
) -> Option<String> {
    match root.file_name().and_then(|value| value.to_str()) {
        Some(value) => Some(value.to_owned()),
        None => {
            issues.push(PluginDiscoveryIssue::new(
                root.to_path_buf(),
                PluginDiscoveryIssueKind::InvalidInstallPath,
                None,
                format!("plugin {level} directory name must be valid UTF-8"),
            ));
            None
        }
    }
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
/// host-side checks against the identity `location` asserts.
fn read_and_validate_manifest(
    location: &InstalledPackageLocation,
    manifest_path: &Path,
    logo: Option<String>,
) -> Result<InstalledPlugin, PluginDiscoveryIssue> {
    let package_root = location.package_root.as_path();
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

    let plugin = validate(package_root, &manifest, &location.namespace, logo).map_err(|error| {
        PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidManifest,
            Some(error.field_path().to_string()),
            error.to_string(),
        )
    })?;
    // The name and version come from the manifest while the directory they sit in was written by
    // the host. Disagreement means the tree was tampered with or an install was interrupted
    // mid-commit, and either way the package must not be attributed to one of the two identities:
    // the id decides which data directory, configuration, and Skill rows the package owns.
    if plugin.id.name() != location.directory_name {
        return Err(PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidManifest,
            Some("identifier".to_string()),
            format!(
                "package identifier {} does not match installation directory {}",
                plugin.id.name(),
                location.directory_name,
            ),
        ));
    }
    if plugin.version != location.directory_version {
        return Err(PluginDiscoveryIssue::new(
            manifest_path.to_path_buf(),
            PluginDiscoveryIssueKind::InvalidManifest,
            Some("version".to_string()),
            format!(
                "package version {} does not match installation directory {}",
                plugin.version, location.directory_version,
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
