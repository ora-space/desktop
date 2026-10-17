//! Installs and updates plugin releases by downloading and safely extracting their package.

mod cache;
pub(crate) mod declaration;
pub(crate) mod package;

pub use declaration::ExpectedAgentDeclaration;

use crate::discovery::installed_root;
use crate::limits::package_extract_limits;
use ora_domain::{PluginId, PluginNamespace};
use ora_plugin_manifest::{
    HookTarget, PluginKind, PluginManifest, PluginReleaseSource, ReleaseLocator,
};
use ora_utils::archive::{ArchiveFormat, extract_archive};
use ora_utils::hash;
use ora_utils::http::{
    Checksum, DownloadOptions, DownloadRequest, DownloadSource, HttpDownload, ProgressCallback,
};
use semver::Version;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Reports why a plugin release could not be installed.
#[derive(Debug, Error)]
pub enum InstallError {
    /// The release package could not be fetched or verified.
    #[error("failed to download release package: {0}")]
    Download(#[source] Box<ora_utils::http::DownloadError>),
    /// The manifest does not declare a downloadable release package.
    #[error("plugin manifest declares no release package to download")]
    MissingRelease,
    /// The release package failed the safe-extraction step.
    #[error("failed to extract release package into {path}: {source}")]
    Extract {
        path: PathBuf,
        #[source]
        source: ora_utils::archive::ArchiveError,
    },
    /// The package does not contain an `orax.toml` manifest at its root.
    #[error("plugin package does not contain orax.toml at its root")]
    MissingManifest,
    /// The in-package `orax.toml` could not be parsed or validated.
    #[error("plugin package manifest is invalid: {0}")]
    InvalidManifest(#[from] ora_plugin_manifest::ManifestError),
    /// The extracted package is invalid at one manifest field: a host-side requirement for its
    /// kind, its identity relative to the marketplace listing, or its manifest source.
    #[error("plugin package is invalid at `{field_path}`: {message}")]
    InvalidPackage { field_path: String, message: String },
    /// The imported archive digest does not match the digest the in-archive manifest declares.
    #[error("imported archive digest {actual} does not match the declared sha256 {expected}")]
    ChecksumMismatch { expected: String, actual: String },
    /// A targeted archive self-declares a different target than the one the release selected.
    #[error(
        "installed artifact target {artifact} does not match the selected release target {release}"
    )]
    TargetMismatch { release: String, artifact: String },
    /// A targeted archive is missing the in-package `[artifact]` self-declaration required to
    /// prove host compatibility independently of marketplace metadata.
    #[error("targeted archive does not declare an [artifact] target")]
    MissingArtifactTarget,
    /// The release does not provide an artifact for the requested host target.
    #[error("release has no artifact for target {target}")]
    NoArtifactForTarget { target: String },
    /// The compiled host is not a supported plugin target, so a targeted release cannot be
    /// selected and a Hook archive cannot be matched against the machine it would run on.
    #[error("current host is not a supported plugin target")]
    UnsupportedHost,
    /// A plugin with the same namespace, name, and version is already installed.
    #[error(
        "a plugin with namespace `{namespace}` name `{name}` version `{version}` is already installed at {path}"
    )]
    AlreadyInstalled {
        path: PathBuf,
        namespace: String,
        name: String,
        version: String,
    },
    /// A prerequisite directory could not be prepared.
    #[error("failed to prepare {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<ora_utils::http::DownloadError> for InstallError {
    fn from(source: ora_utils::http::DownloadError) -> Self {
        Self::Download(Box::new(source))
    }
}

impl InstallError {
    pub(crate) fn invalid_package(source: crate::validation::ManifestValidationError) -> Self {
        Self::invalid_field(source.field_path(), source.to_string())
    }

    /// Builds a package-content failure the installer detects without the validation crate.
    ///
    /// A manifest source that cannot be read and an identity that contradicts the listing are the
    /// same kind of user-facing problem as a failed host-side check — "this package is wrong at
    /// field X" — so they carry that shape instead of growing one variant per detection site.
    pub(crate) fn invalid_field(field_path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidPackage {
            field_path: field_path.into(),
            message: message.into(),
        }
    }
}

/// Carries the resolved download source and digest for one release, after target selection.
///
/// The backend resolves this from a release manifest before handing it to the installer, so the
/// installer never names a transport or target-selection policy. A universal release carries no
/// target; a targeted release carries the selected target so the installer can verify it against
/// the package's self-declared artifact target after extraction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedReleaseSource {
    download: DownloadSource,
    sha256: [u8; 32],
    target: Option<HookTarget>,
}

impl ResolvedReleaseSource {
    /// Builds a resolved source for a universal release.
    pub fn universal(download: DownloadSource, sha256: [u8; 32]) -> Self {
        Self {
            download,
            sha256,
            target: None,
        }
    }

    /// Builds a resolved source for a targeted release, carrying the selected target.
    pub fn targeted(download: DownloadSource, sha256: [u8; 32], target: HookTarget) -> Self {
        Self {
            download,
            sha256,
            target: Some(target),
        }
    }

    /// Returns the download source to pass to the downloader.
    pub fn download(&self) -> &DownloadSource {
        &self.download
    }

    /// Returns the SHA-256 digest bytes that verify the downloaded archive.
    pub fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }

    /// Returns the selected target, present only for a targeted release.
    pub fn target(&self) -> Option<&HookTarget> {
        self.target.as_ref()
    }
}

/// Distinguishes a supported host triple from a compiled host that cannot run targeted plugins.
///
/// Universal releases ignore this value. Targeted marketplace selection and Hook local import
/// require [`HostTarget::Triple`]. [`HostTarget::Unsupported`] is the explicit "this binary is
/// not a plugin host" case, not "universal does not need a host".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostTarget<'a> {
    /// The compiled host is not a supported plugin target.
    Unsupported,
    /// The host's canonical Rust target triple.
    Triple(&'a HookTarget),
}

impl<'a> HostTarget<'a> {
    /// Maps `current_host_target()` into a self-documenting host capability.
    pub fn from_option(target: Option<&'a HookTarget>) -> Self {
        match target {
            Some(target) => Self::Triple(target),
            None => Self::Unsupported,
        }
    }
}

/// Selects the release source matching `host_target` from a manifest's `release_source()`.
///
/// A universal release resolves to the single URL/digest pair regardless of host. A targeted
/// release requires a supported host and resolves to the one exact-matching target, returning
/// `UnsupportedHost` when the compiled host is not a plugin target and `NoArtifactForTarget`
/// when that host has no matching artifact, so the caller can reject before download.
pub fn select_release(
    manifest: &PluginManifest,
    host_target: HostTarget<'_>,
) -> Result<ResolvedReleaseSource, InstallError> {
    match manifest.release_source() {
        Some(PluginReleaseSource::Universal { url, sha256 }) => Ok(
            ResolvedReleaseSource::universal(download_source(url), *sha256.as_bytes()),
        ),
        Some(PluginReleaseSource::Targets(targets)) => {
            let HostTarget::Triple(host_target) = host_target else {
                return Err(InstallError::UnsupportedHost);
            };
            let entry = targets
                .iter()
                .find(|entry| entry.target() == host_target)
                .ok_or_else(|| InstallError::NoArtifactForTarget {
                    target: host_target.to_string(),
                })?;
            Ok(ResolvedReleaseSource::targeted(
                download_source(entry.url()),
                *entry.sha256().as_bytes(),
                entry.target().clone(),
            ))
        }
        None => Err(InstallError::MissingRelease),
    }
}

/// Maps a marketplace locator onto the download source the installer injects.
fn download_source(locator: &ReleaseLocator) -> DownloadSource {
    match locator {
        ReleaseLocator::Https(url) => DownloadSource::Url(url.as_url().clone()),
        ReleaseLocator::ObjectKey(key) => DownloadSource::S3 {
            key: key.as_str().to_owned(),
        },
    }
}

/// Derives the declaration expectation a release listing declares, if any.
///
/// The copy path is marketplace-side only: what an install must prove is that the package's
/// declaration bytes carry the digest the listing associated, not where the sync read a copy.
fn expected_agent_declaration(manifest: &PluginManifest) -> Option<ExpectedAgentDeclaration> {
    manifest.agent_configuration().map(|association| {
        ExpectedAgentDeclaration::new(
            association.schema_version(),
            *association.declaration_sha256(),
        )
    })
}

/// Reports why one plugin release could not be updated.
#[derive(Debug, Error)]
pub enum UpdateError {
    /// The new release could not be downloaded, verified, or materialized.
    #[error("failed to install the updated release: {0}")]
    Install(#[from] InstallError),
    /// No installed package exists for the plugin, so there is nothing to update.
    #[error("plugin `{id}` is not installed")]
    NotFound { id: String },
    /// The marketplace still publishes the version that is already installed.
    #[error("plugin `{id}` version `{version}` is already up to date")]
    AlreadyUpToDate { id: String, version: String },
    /// The marketplace publishes a version older than the installed one.
    #[error(
        "marketplace version `{available}` is older than installed version `{installed}` for plugin `{id}`"
    )]
    Downgrade {
        id: String,
        installed: String,
        available: String,
    },
    /// A stale version directory could not be removed after the new version landed.
    #[error("failed to remove stale plugin version {path} for `{id}`: {source}")]
    Retire {
        id: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Describes one package materialized from a release archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    /// The package directory below `<data-dir>/plugins/installed/<namespace>/<name>/<version>`.
    pub package_dir: PathBuf,
    /// The installed plugin's identity: the name segment comes from the in-package manifest, the
    /// namespace from the installing source. The manifest never names its own namespace.
    pub id: PluginId,
}

/// Orchestrates one plugin installation and stays backend-agnostic.
///
/// The downloader is injected so the orchestration never names a concrete transport; production
/// wiring supplies a network downloader while tests (and offline installs) use the local one.
#[derive(Clone)]
pub struct Installer<D> {
    pub(crate) downloader: D,
}

impl<D> Installer<D>
where
    D: HttpDownload,
{
    /// Creates an installer that downloads every release through `downloader`.
    pub fn new(downloader: D) -> Self {
        Self { downloader }
    }

    /// Downloads `manifest`'s release into the cache, verifies its digest, and extracts it into
    /// `<data-dir>/plugins/installed/<namespace>/<name>/<version>`, returning that package
    /// directory.
    ///
    /// `namespace` is the identity of the marketplace source the release was resolved from, and
    /// the caller is the only party that knows it: a package author cannot know which source a
    /// user installed their plugin through, so the manifest does not declare it. Writing it as
    /// the first directory level is what makes the installed tree, and nothing else, the record
    /// of where a plugin came from.
    ///
    /// For a universal release, `source` carries the download URL and the manifest's `sha256`
    /// verifies the bytes. For a targeted release, `source` is the resolved target-specific
    /// artifact (selected by the caller against `host_target`) and the matching target digest
    /// verifies the bytes; the installer then confirms the extracted package's artifact target
    /// equals the selected release target so a wrong-architecture archive never installs as valid.
    pub async fn install(
        &self,
        manifest: &PluginManifest,
        namespace: &PluginNamespace,
        source: ResolvedReleaseSource,
        data_dir: &Path,
    ) -> Result<PathBuf, InstallError> {
        self.install_package(
            manifest, namespace, source, data_dir, /*progress*/ None,
        )
        .await
    }

    /// Installs a release while forwarding byte-level network download progress to the caller.
    pub async fn install_with_progress(
        &self,
        manifest: &PluginManifest,
        namespace: &PluginNamespace,
        source: ResolvedReleaseSource,
        data_dir: &Path,
        progress: ProgressCallback,
    ) -> Result<PathBuf, InstallError> {
        self.install_package(manifest, namespace, source, data_dir, Some(progress))
            .await
    }

    /// Shares the atomic install path between callers that do and do not observe the transfer.
    ///
    /// The downloaded archive is a transfer artifact rather than state: it is deleted whether the
    /// install commits or fails, and any archive an earlier run left behind is swept before this
    /// one downloads its own. A failed install therefore leaves the installed tree exactly as it
    /// found it — no version directory, no empty package parent, and no cached `.orax` — while a
    /// successful one leaves only the installed package beside the derived marketplace index.
    async fn install_package(
        &self,
        manifest: &PluginManifest,
        namespace: &PluginNamespace,
        source: ResolvedReleaseSource,
        data_dir: &Path,
        progress: Option<ProgressCallback>,
    ) -> Result<PathBuf, InstallError> {
        cache::sweep_leftovers(data_dir)?;
        let name = manifest.name();
        let version = manifest.version().to_string();
        let package_parent = installed_root(data_dir)
            .join(namespace.as_str())
            .join(name.as_str());
        let package_dir = package_parent.join(&version);
        // The already-installed check runs before the transfer so re-installing a version never
        // downloads a package it will refuse to unpack. `update_package` has already compared
        // versions, so a directory here belongs to an unrelated install that committed first.
        if package_dir.exists() {
            return Err(InstallError::AlreadyInstalled {
                path: package_dir,
                namespace: namespace.as_str().to_owned(),
                name: name.as_str().to_owned(),
                version,
            });
        }
        let archive_path =
            cache::release_archive_path(data_dir, namespace, name.as_str(), &version);
        if let Err(error) = self
            .download_package(&archive_path, source.clone(), progress)
            .await
        {
            // A verified archive is renamed into place only after its digest matches, so a failed
            // transfer normally leaves nothing behind; the removal covers the window between that
            // rename and the failure.
            cache::discard_downloaded_archive(&archive_path);
            return Err(error);
        }
        let outcome = package::materialize_package(
            &archive_path,
            &package_dir,
            &package_parent,
            namespace,
            manifest,
            source.target(),
            expected_agent_declaration(manifest).as_ref(),
        );
        cache::discard_downloaded_archive(&archive_path);
        match outcome {
            Ok(()) => Ok(package_dir),
            Err(error) => {
                cache::remove_empty_directory(&package_parent);
                Err(error)
            }
        }
    }

    /// Updates one installed plugin to `manifest`'s version by downloading, verifying, and
    /// extracting the new release, then retiring every older version directory.
    ///
    /// The currently installed version is derived from the highest valid SemVer directory below
    /// `<data-dir>/plugins/installed/<namespace>/<name>`, so the marketplace cannot silently
    /// downgrade a package or re-materialize an identical release. Only a verified package is
    /// ever committed, and stale versions are removed after the new one is on disk so a failed
    /// download leaves the previous installation untouched. Targeted Hook updates reuse the same
    /// `ResolvedReleaseSource` host-selection path as a first install.
    pub async fn update(
        &self,
        manifest: &PluginManifest,
        namespace: &PluginNamespace,
        source: ResolvedReleaseSource,
        data_dir: &Path,
    ) -> Result<InstalledPackage, UpdateError> {
        self.update_package(
            manifest, namespace, source, data_dir, /*progress*/ None,
        )
        .await
    }

    /// Updates a release while forwarding byte-level network download progress to the caller.
    pub async fn update_with_progress(
        &self,
        manifest: &PluginManifest,
        namespace: &PluginNamespace,
        source: ResolvedReleaseSource,
        data_dir: &Path,
        progress: ProgressCallback,
    ) -> Result<InstalledPackage, UpdateError> {
        self.update_package(manifest, namespace, source, data_dir, Some(progress))
            .await
    }

    /// Shares update validation and retirement between observed and unobserved transfers.
    async fn update_package(
        &self,
        manifest: &PluginManifest,
        namespace: &PluginNamespace,
        source: ResolvedReleaseSource,
        data_dir: &Path,
        progress: Option<ProgressCallback>,
    ) -> Result<InstalledPackage, UpdateError> {
        let name = manifest.name();
        let id = installed_id(namespace, name.as_str());
        let plugin_root = installed_root(data_dir)
            .join(namespace.as_str())
            .join(name.as_str());
        let latest_installed = match latest_installed_version(&plugin_root) {
            Ok(latest) => latest,
            // A missing name root means the plugin was never installed, which is a distinct
            // outcome from a stale-version removal failure.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(UpdateError::Retire {
                    id: id.canonical(),
                    path: plugin_root.clone(),
                    source,
                });
            }
        };
        let Some(latest_installed) = latest_installed else {
            return Err(UpdateError::NotFound { id: id.canonical() });
        };
        if manifest.version() == &latest_installed {
            return Err(UpdateError::AlreadyUpToDate {
                id: id.canonical(),
                version: latest_installed.to_string(),
            });
        }
        if manifest.version() < &latest_installed {
            return Err(UpdateError::Downgrade {
                id: id.canonical(),
                installed: latest_installed.to_string(),
                available: manifest.version().to_string(),
            });
        }
        let package_dir = self
            .install_package(manifest, namespace, source, data_dir, progress)
            .await?;
        retire_stale_versions(&id.canonical(), &plugin_root, &package_dir)?;
        Ok(InstalledPackage { package_dir, id })
    }

    /// Imports an already-downloaded release archive from `archive_path` into the installed tree.
    ///
    /// A locally imported package has no marketplace source, so there is no URL to derive an
    /// identity from and it is installed under the reserved `local` namespace.
    ///
    /// Unlike a marketplace install, the manifest lives inside the archive, so this extracts into
    /// a disposable staging directory first, reads and validates the in-archive `orax.toml`, and
    /// only then moves the verified tree into
    /// `<data-dir>/plugins/installed/local/<name>/<version>`. A `sha256` declared by the
    /// in-archive manifest is checked against the archive before anything is committed. A Hook
    /// archive must self-declare `[artifact]` and that target must match `host_target`; other
    /// kinds ignore the host. [`HostTarget::Unsupported`] refuses Hook imports and leaves
    /// universal packages installable.
    pub fn install_local(
        &self,
        archive_path: &Path,
        data_dir: &Path,
        host_target: HostTarget<'_>,
    ) -> Result<InstalledPackage, InstallError> {
        let plugins_dir = data_dir.join("plugins");
        // Importing may target a profile that never synced the marketplace, so the plugins
        // root is created here; a missing `plugins/` directory would otherwise fail the
        // staging temp-dir reservation below with a confusing NotFound.
        std::fs::create_dir_all(&plugins_dir).map_err(|source| InstallError::Io {
            path: plugins_dir.clone(),
            source,
        })?;
        let staging = tempfile::tempdir_in(&plugins_dir).map_err(|source| InstallError::Io {
            path: plugins_dir.clone(),
            source,
        })?;
        extract_archive(
            ArchiveFormat::Zip,
            archive_path,
            staging.path(),
            &package_extract_limits(),
        )
        .map_err(|source| InstallError::Extract {
            path: staging.path().to_path_buf(),
            source,
        })?;

        let manifest_path = staging.path().join("orax.toml");
        if !manifest_path.is_file() {
            return Err(InstallError::MissingManifest);
        }
        let manifest_source =
            std::fs::read_to_string(&manifest_path).map_err(|source| InstallError::Io {
                path: manifest_path,
                source,
            })?;
        let manifest = PluginManifest::parse_installed(&manifest_source)?;

        // The digest is self-declared by the package, so verifying it here only catches a
        // corrupt or degraded archive during transit or storage; it provides no anti-tamper
        // guarantee, since the digest and the package ship together.
        if let Some(digest) = manifest.sha256() {
            let expected = digest.to_string();
            let actual = hash::sha256_file(archive_path).map_err(|source| InstallError::Io {
                path: archive_path.to_path_buf(),
                source,
            })?;
            if actual != expected {
                return Err(InstallError::ChecksumMismatch { expected, actual });
            }
        }

        let namespace = PluginNamespace::local();
        let name = manifest.name();
        let version = manifest.version().to_string();
        let destination = installed_root(data_dir)
            .join(namespace.as_str())
            .join(name.as_str())
            .join(&version);
        if destination.exists() {
            return Err(InstallError::AlreadyInstalled {
                path: destination,
                namespace: namespace.as_str().to_owned(),
                name: name.as_str().to_owned(),
                version,
            });
        }
        crate::validation::validate(staging.path(), &manifest, &namespace, /*logo*/ None)
            .map_err(InstallError::invalid_package)?;
        // Any package that self-declares a target must match the host it is imported onto, and an
        // unverifiable host fails closed rather than skipping the match. Requiring the declaration
        // stays Hook-only: a Hook is its native binary, while an Agent that resolves its CLI from
        // PATH is a legitimate universal package with no target to check.
        if let Some(artifact) = manifest.artifact() {
            let HostTarget::Triple(host_target) = host_target else {
                return Err(InstallError::UnsupportedHost);
            };
            if artifact.target() != host_target {
                return Err(InstallError::TargetMismatch {
                    release: host_target.to_string(),
                    artifact: artifact.target().to_string(),
                });
            }
        } else if matches!(manifest.kind(), PluginKind::Hook) {
            return Err(InstallError::MissingArtifactTarget);
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| InstallError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        // The staged tree is fully materialized and validated, so one rename commits the package
        // atomically; the disposable staging root is cleaned up when it drops afterwards.
        std::fs::rename(staging.path(), &destination).map_err(|source| InstallError::Io {
            path: destination.clone(),
            source,
        })?;

        Ok(InstalledPackage {
            package_dir: destination,
            id: installed_id(&namespace, name.as_str()),
        })
    }

    /// Fetches and verifies one release archive into `archive_path`.
    ///
    /// The path is supplied by the caller because the caller also owns the archive's lifetime: it
    /// sweeps leftovers before the transfer and deletes the file on every exit path.
    async fn download_package(
        &self,
        archive_path: &Path,
        source: ResolvedReleaseSource,
        progress: Option<ProgressCallback>,
    ) -> Result<(), InstallError> {
        if let Some(cache_dir) = archive_path.parent() {
            std::fs::create_dir_all(cache_dir).map_err(|source| InstallError::Io {
                path: cache_dir.to_path_buf(),
                source,
            })?;
        }
        let digest = source.sha256();
        let request = DownloadRequest {
            source: source.download().clone(),
            destination: archive_path.to_path_buf(),
            checksum: Some(Checksum::sha256(digest.to_vec())),
            options: DownloadOptions::default(),
            progress,
            cancel: None,
        };
        self.downloader.download(request).await?;
        Ok(())
    }
}

/// Pairs a host-supplied namespace with a manifest-validated name into one installed identity.
///
/// Both halves already satisfy the id grammar — the namespace is a validated segment and the
/// manifest name is a strict subset of it — so the fallback only keeps the function total.
fn installed_id(namespace: &PluginNamespace, name: &str) -> PluginId {
    PluginId::new(namespace.clone(), name).unwrap_or_else(|error| {
        unreachable!("validated package segments form a plugin id: {error}")
    })
}

/// Returns the highest valid SemVer version directory below `plugin_root`, if any.
///
/// Directory names that are not valid SemVer are ignored here exactly as discovery treats them:
/// they cannot decide whether an update is a no-op or a downgrade, and they are retired along
/// with every other stale version once a new release lands.
fn latest_installed_version(plugin_root: &Path) -> std::io::Result<Option<Version>> {
    let mut latest = None;
    for entry in std::fs::read_dir(plugin_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Ok(version) = Version::parse(&entry.file_name().to_string_lossy())
            && (latest.as_ref().is_none_or(|installed| version > *installed))
        {
            latest = Some(version);
        }
    }
    Ok(latest)
}

/// Removes every version directory below `plugin_root` except the one that was just installed.
///
/// The plugin name root is derived from manifest-validated segments and the retained directory is
/// the exact path the installer just committed, so only sibling version directories can be
/// touched. A removal failure is reported after the new version is already committed; the caller
/// keeps the installed plugin usable and can retry cleanup independently.
fn retire_stale_versions(id: &str, plugin_root: &Path, retain: &Path) -> Result<(), UpdateError> {
    for entry in std::fs::read_dir(plugin_root).map_err(|source| UpdateError::Retire {
        id: id.to_owned(),
        path: plugin_root.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| UpdateError::Retire {
            id: id.to_owned(),
            path: plugin_root.to_path_buf(),
            source,
        })?;
        if entry.path() == retain {
            continue;
        }
        std::fs::remove_dir_all(entry.path()).map_err(|source| UpdateError::Retire {
            id: id.to_owned(),
            path: entry.path(),
            source,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        HostTarget, InstallError, InstalledPackage, Installer, ResolvedReleaseSource, UpdateError,
        installed_id,
    };
    use futures::executor::block_on;
    use ora_domain::PluginNamespace;
    use ora_plugin_manifest::{HookTarget, PluginManifest};
    use ora_utils::http::{
        DownloadError, DownloadOutcome, DownloadRequest, DownloadSource, HttpDownload,
        LocalFileDownloader, Progress,
    };
    use pretty_assertions::assert_eq;
    use sha2::{Digest, Sha256};
    use std::fs::{self, File};
    use std::future::Future;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    /// Emits a deterministic snapshot before delegating to the real local-file copy path.
    #[derive(Clone, Copy)]
    struct ProgressReportingLocalDownloader;

    impl HttpDownload for ProgressReportingLocalDownloader {
        fn download(
            &self,
            request: DownloadRequest,
        ) -> impl Future<Output = Result<DownloadOutcome, DownloadError>> + Send {
            async move {
                if let Some(progress) = &request.progress {
                    progress(Progress {
                        bytes: 3,
                        total: Some(4),
                    });
                }
                LocalFileDownloader.download(request).await
            }
        }
    }

    /// Computes the SHA-256 digest of a file as raw bytes for the manifest.
    fn sha256_file(path: &Path) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&fs::read(path).unwrap());
        hasher.finalize().into()
    }

    /// Renders a digest as lowercase hex without aggregating per-byte formatting.
    fn hex(bytes: [u8; 32]) -> String {
        const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(64);
        for byte in bytes {
            output.push(char::from(HEX_DIGITS[(byte >> 4) as usize]));
            output.push(char::from(HEX_DIGITS[(byte & 0x0f) as usize]));
        }
        output
    }

    /// Returns the Windows x86_64 host triple used by Hook local-import tests.
    fn windows_host() -> HookTarget {
        HookTarget::parse("x86_64-pc-windows-msvc").unwrap()
    }

    /// Writes an orax-shaped zip package with the given files.
    fn write_orax_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut writer = ZipWriter::new(File::create(path).unwrap());
        let options = SimpleFileOptions::default();
        for (name, data) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    /// Builds a complete installed-form manifest for one agent package.
    fn installed_agent_manifest(name: &str, version: &str) -> String {
        format!(
            "resolver = 1\nidentifier = \"{name}\"\nkind = \"agent\"\nversion = \"{version}\"\ndescription = \"A test plugin\"\n"
        )
    }

    /// Returns the cache path one release archive is downloaded into.
    fn cache_archive_path(data_dir: &Path, namespace: &str, name: &str, version: &str) -> PathBuf {
        data_dir
            .join("plugins")
            .join("cache")
            .join(namespace)
            .join(format!("{name}-{version}.orax"))
    }

    /// Asserts a failed install left no package, no empty package parent, and no cached archive.
    fn assert_no_install_residue(data_dir: &Path, namespace: &str, name: &str, version: &str) {
        let package_parent = data_dir
            .join("plugins")
            .join("installed")
            .join(namespace)
            .join(name);
        assert!(
            !package_parent.join(version).exists(),
            "no package directory is committed"
        );
        assert!(
            !package_parent.exists(),
            "the empty package parent directory is removed"
        );
        assert!(
            !cache_archive_path(data_dir, namespace, name, version).exists(),
            "the downloaded archive is deleted"
        );
    }

    /// Builds an agent orax manifest whose sha256 matches `digest`.
    fn manifest_with_digest(name: &str, version: &str, digest: [u8; 32]) -> String {
        manifest_with_kind_digest(name, version, "agent", digest)
    }

    /// Builds a marketplace manifest for `kind` whose sha256 matches `digest`.
    fn manifest_with_kind_digest(
        name: &str,
        version: &str,
        kind: &str,
        digest: [u8; 32],
    ) -> String {
        format!(
            "resolver = 1\nidentifier = \"{name}\"\nkind = \"{kind}\"\nversion = \"{version}\"\ndescription = \"A test plugin\"\nurl = \"https://example.com/{name}.orax\"\nsha256 = \"{}\"\n",
            hex(digest)
        )
    }

    /// Packs `files` as the marketplace release `official/weather` 1.0.0, listed with
    /// `listing_kind` and the listing's own description, and installs it into `data_dir`.
    ///
    /// The listing is built from the archive's real digest so every failure the caller observes
    /// comes from the package content rather than from transfer verification.
    fn install_weather_release(
        data_dir: &Path,
        listing_kind: &str,
        files: &[(&str, &[u8])],
    ) -> Result<PathBuf, InstallError> {
        let release_path = data_dir.join("pkg.orax");
        write_orax_zip(&release_path, files);
        let digest = sha256_file(&release_path);
        let manifest = PluginManifest::parse(&manifest_with_kind_digest(
            "weather",
            "1.0.0",
            listing_kind,
            digest,
        ))
        .unwrap();
        block_on(Installer::new(LocalFileDownloader).install(
            &manifest,
            &PluginNamespace::official(),
            ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            data_dir,
        ))
    }

    /// Verifies a full local install: cache download, checksum, extraction, and cache cleanup.
    #[test]
    fn installs_local_release_end_to_end() {
        let temp_dir = TempDir::new().unwrap();
        let package_dir = install_weather_release(
            temp_dir.path(),
            "agent",
            &[
                (
                    "orax.toml",
                    installed_agent_manifest("weather", "1.0.0").as_bytes(),
                ),
                ("main.js", b"export {};\n".as_slice()),
                ("logo.svg", b"<svg/>".as_slice()),
            ],
        )
        .unwrap();

        let expected_package = temp_dir
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join("weather")
            .join("1.0.0");
        assert_eq!(package_dir, expected_package);
        assert!(expected_package.join("orax.toml").exists());
        assert!(expected_package.join("main.js").exists());
        assert!(expected_package.join("logo.svg").exists());
        assert!(
            !cache_archive_path(temp_dir.path(), "official", "weather", "1.0.0").exists(),
            "the downloaded archive is deleted once the install commits"
        );
        assert!(
            !temp_dir
                .path()
                .join("plugins")
                .join("cache")
                .join("official")
                .exists(),
            "the emptied namespace directory leaves the cache holding only the registry index"
        );
    }

    /// A marketplace release whose archive ships no `orax.toml` is refused while it is still
    /// staged: the error names the missing file, nothing is committed, and the downloaded archive
    /// is deleted.
    #[test]
    fn rejects_a_release_without_an_in_package_manifest() {
        let temp_dir = TempDir::new().unwrap();
        let error = install_weather_release(
            temp_dir.path(),
            "agent",
            &[("main.js", b"export {};\n".as_slice())],
        )
        .unwrap_err();

        assert!(matches!(error, InstallError::MissingManifest));
        assert_no_install_residue(temp_dir.path(), "official", "weather", "1.0.0");
    }

    /// An archive entry named `orax.toml` that is not a regular file is refused instead of being
    /// read, so a package cannot substitute a directory for its manifest.
    #[test]
    fn rejects_a_manifest_entry_that_is_not_a_regular_file() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("pkg.orax");
        let mut writer = ZipWriter::new(File::create(&release_path).unwrap());
        let options = SimpleFileOptions::default();
        // A directory entry is only materialized when it has to hold something, so the substituted
        // manifest carries one child file.
        writer
            .start_file("orax.toml/not-a-manifest.txt", options)
            .unwrap();
        writer.write_all(b"not a manifest").unwrap();
        writer.start_file("main.js", options).unwrap();
        writer.write_all(b"export {};\n").unwrap();
        writer.finish().unwrap();
        let digest = sha256_file(&release_path);
        let manifest =
            PluginManifest::parse(&manifest_with_digest("weather", "1.0.0", digest)).unwrap();

        let error = block_on(Installer::new(LocalFileDownloader).install(
            &manifest,
            &PluginNamespace::official(),
            ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            temp_dir.path(),
        ))
        .unwrap_err();

        match error {
            InstallError::InvalidPackage { field_path, .. } => {
                assert_eq!(field_path, "orax.toml");
            }
            other => panic!("expected an invalid manifest source, got {other:?}"),
        }
        assert_no_install_residue(temp_dir.path(), "official", "weather", "1.0.0");
    }

    /// A package whose manifest omits a required field is refused with that field's path, so the
    /// user learns which field to fix instead of seeing an internal error.
    #[test]
    fn rejects_a_package_manifest_missing_a_required_field() {
        let temp_dir = TempDir::new().unwrap();
        let error = install_weather_release(
            temp_dir.path(),
            "agent",
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"weather\"\nkind = \"agent\"\nversion = \"1.0.0\"\n"
                        .as_slice(),
                ),
                ("main.js", b"export {};\n".as_slice()),
            ],
        )
        .unwrap_err();

        match error {
            InstallError::InvalidManifest(source) => assert_eq!(
                source.field_path(),
                Some("description".to_owned()),
                "{source}"
            ),
            other => panic!("expected a structural manifest error, got {other:?}"),
        }
        assert_no_install_residue(temp_dir.path(), "official", "weather", "1.0.0");
    }

    /// A package whose manifest is not valid TOML is refused while staged, and the failure names
    /// no field because a syntax error belongs to the document as a whole.
    #[test]
    fn rejects_a_package_manifest_that_is_not_valid_toml() {
        let temp_dir = TempDir::new().unwrap();
        let error = install_weather_release(
            temp_dir.path(),
            "agent",
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"weather\"\nkind = \"agent\"\nversion = \ndescription = \"A test plugin\"\n"
                        .as_slice(),
                ),
                ("main.js", b"export {};\n".as_slice()),
            ],
        )
        .unwrap_err();

        match error {
            InstallError::InvalidManifest(source) => {
                assert_eq!(source.field_path(), None, "{source}");
            }
            other => panic!("expected a TOML syntax error, got {other:?}"),
        }
        assert_no_install_residue(temp_dir.path(), "official", "weather", "1.0.0");
    }

    /// Descriptive metadata is deliberately outside the identity comparison: a package whose
    /// title, description, homepage, and license differ from its listing still installs, and
    /// discovery reports the package's own text because that is what the installed tree holds.
    #[test]
    fn installs_a_package_whose_descriptive_metadata_differs_from_the_listing() {
        let temp_dir = TempDir::new().unwrap();
        let package_dir = install_weather_release(
            temp_dir.path(),
            "agent",
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"weather\"\ntitle = \"Weather Pro\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Forecasts shipped by the package\"\nhomepage = \"https://example.com/weather\"\nlicense = \"MIT\"\n"
                        .as_slice(),
                ),
                ("main.js", b"export {};\n".as_slice()),
            ],
        )
        .unwrap();

        let discovered = crate::PluginManager::discover(temp_dir.path());
        let [plugin] = discovered.installed_plugins() else {
            panic!(
                "expected exactly the installed package, got {:?}",
                discovered.installed_plugins()
            );
        };
        assert_eq!(
            (
                plugin.package_root.clone(),
                plugin.display_name.as_str(),
                plugin.description.as_str(),
                plugin.homepage.as_deref(),
                plugin.license.as_deref(),
            ),
            (
                package_dir,
                "Weather Pro",
                "Forecasts shipped by the package",
                Some("https://example.com/weather"),
                Some("MIT"),
            )
        );
    }

    /// A package whose kind-specific contribution is malformed is refused at install time with
    /// the file that carries it, instead of landing on disk and failing later during discovery.
    #[test]
    fn rejects_a_package_whose_kind_contribution_is_malformed() {
        let temp_dir = TempDir::new().unwrap();
        let error = install_weather_release(
            temp_dir.path(),
            "mcp",
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"weather\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"A test plugin\"\n"
                        .as_slice(),
                ),
                (
                    "assets/config.json",
                    br#"{"schemaVersion":1,"transport":{"type":"http"}}"#.as_slice(),
                ),
            ],
        )
        .unwrap_err();

        match error {
            InstallError::InvalidPackage { field_path, .. } => {
                assert_eq!(field_path, "assets/config.json");
            }
            other => panic!("expected an invalid MCP contribution, got {other:?}"),
        }
        assert_no_install_residue(temp_dir.path(), "official", "weather", "1.0.0");
    }

    /// A package whose own manifest contradicts the listing identity never installs: identity
    /// decides the installed directory and the plugin's data, so both spellings cannot be honored.
    #[test]
    fn rejects_packages_that_contradict_the_listing_identity() {
        let cases = [
            (
                "identifier",
                "resolver = 1\nidentifier = \"weather-service\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"A test plugin\"\n",
                "package identifier `weather-service` does not match the marketplace listing identifier `weather`",
            ),
            (
                "version",
                "resolver = 1\nidentifier = \"weather\"\nkind = \"agent\"\nversion = \"1.1.0\"\ndescription = \"A test plugin\"\n",
                "package version `1.1.0` does not match the marketplace listing version `1.0.0`",
            ),
            (
                "kind",
                "resolver = 1\nidentifier = \"weather\"\nkind = \"webview\"\nversion = \"1.0.0\"\ndescription = \"A test plugin\"\n\n[webview]\nstart_url = \"https://example.com\"\nallowed_origins = [\"https://example.com\"]\n",
                "package kind `webview` does not match the marketplace listing kind `agent`",
            ),
        ];

        for (field, in_package_manifest, expected_message) in cases {
            let temp_dir = TempDir::new().unwrap();
            let error = install_weather_release(
                temp_dir.path(),
                "agent",
                &[
                    ("orax.toml", in_package_manifest.as_bytes()),
                    ("main.js", b"export {};\n".as_slice()),
                ],
            )
            .unwrap_err();

            match error {
                InstallError::InvalidPackage {
                    field_path,
                    message,
                } => assert_eq!(
                    (field_path, message),
                    (field.to_owned(), expected_message.to_owned()),
                    "identity mismatch in `{field}`"
                ),
                other => panic!("expected an identity mismatch in `{field}`, got {other:?}"),
            }
            assert_no_install_residue(temp_dir.path(), "official", "weather", "1.0.0");
        }
    }

    /// An install starts from a cache that holds only the derived marketplace index: leftovers
    /// from a crashed run are swept, while the index every listing read depends on is untouched.
    #[test]
    fn sweeps_leftover_archives_without_touching_the_registry_index() {
        let temp_dir = TempDir::new().unwrap();
        let cache_root = temp_dir.path().join("plugins").join("cache");
        // One archive in the pre-namespace layout, one in the current layout, the durable index,
        // and the temporary file of a download that is still in progress.
        std::fs::create_dir_all(cache_root.join("thirdparty")).unwrap();
        std::fs::write(cache_root.join("weather-1.0.0.orax"), b"stale").unwrap();
        std::fs::write(
            cache_root.join("thirdparty").join("weather-1.0.0.orax"),
            b"stale",
        )
        .unwrap();
        std::fs::write(
            cache_root.join("registry_index.json"),
            b"{\"version\":\"1.0\"}",
        )
        .unwrap();
        std::fs::write(cache_root.join("weather-1.0.0.orax.tmp"), b"partial").unwrap();

        install_weather_release(
            temp_dir.path(),
            "agent",
            &[
                (
                    "orax.toml",
                    installed_agent_manifest("weather", "1.0.0").as_bytes(),
                ),
                ("main.js", b"export {};\n".as_slice()),
            ],
        )
        .unwrap();

        assert!(
            !cache_root.join("weather-1.0.0.orax").exists(),
            "the pre-namespace leftover is swept"
        );
        assert!(
            !cache_root.join("thirdparty").exists(),
            "an emptied namespace directory is removed"
        );
        assert_eq!(
            std::fs::read(cache_root.join("registry_index.json")).unwrap(),
            b"{\"version\":\"1.0\"}",
            "the derived marketplace index is never deleted"
        );
        assert_eq!(
            std::fs::read(cache_root.join("weather-1.0.0.orax.tmp")).unwrap(),
            b"partial",
            "a download still in progress is not disturbed"
        );
    }

    /// The same plugin name and version from two sources must not share one cache path: the flat
    /// `<name>-<version>.orax` name did, so a concurrent install from the other source could
    /// overwrite the bytes this install was about to unpack.
    #[test]
    fn keeps_same_name_and_version_releases_from_different_namespaces_apart() {
        let data_dir = Path::new("data");
        let hyphenated_name = super::cache::release_archive_path(
            data_dir,
            &PluginNamespace::parse("foo").unwrap(),
            "bar-baz",
            "1.0.0",
        );
        let hyphenated_namespace = super::cache::release_archive_path(
            data_dir,
            &PluginNamespace::parse("foo-bar").unwrap(),
            "baz",
            "1.0.0",
        );

        assert_ne!(hyphenated_name, hyphenated_namespace);
        assert_eq!(
            hyphenated_name,
            data_dir
                .join("plugins")
                .join("cache")
                .join("foo")
                .join("bar-baz-1.0.0.orax")
        );
        assert_eq!(
            hyphenated_namespace,
            data_dir
                .join("plugins")
                .join("cache")
                .join("foo-bar")
                .join("baz-1.0.0.orax")
        );
    }

    /// A marketplace Skill release is validated and committed with its complete static tree.
    #[test]
    fn installs_marketplace_skill_release_with_required_assets() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("skill-pack.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"identifier = \"ora.skill-pack\"\nnamespace = \"official\"\nkind = \"skill\"\nversion = \"1.0.0\"\ndescription = \"Skill plugin test\"\n".as_slice(),
                ),
                (
                    "assets/review/SKILL.md",
                    b"---\nname: review\ndescription: Reviews code\n---\n".as_slice(),
                ),
            ],
        );
        let manifest = PluginManifest::parse(&manifest_with_kind_digest(
            "ora.skill-pack",
            "1.0.0",
            "skill",
            sha256_file(&release_path),
        ))
        .unwrap();

        let digest = sha256_file(&release_path);
        let package_dir = block_on(Installer::new(LocalFileDownloader).install(
            &manifest,
            &PluginNamespace::official(),
            ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            temp_dir.path(),
        ))
        .expect("install marketplace Skill release");

        assert!(
            package_dir
                .join("assets")
                .join("review")
                .join("SKILL.md")
                .is_file()
        );
        assert!(!package_dir.join("main.js").exists());
    }

    /// A mismatched digest aborts before any package lands in the installed tree.
    #[test]
    fn rejects_checksum_mismatch_and_installs_nothing() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("pkg.orax");
        write_orax_zip(&release_path, &[("orax.toml", b"invalid".as_slice())]);
        let manifest =
            PluginManifest::parse(&manifest_with_digest("weather", "1.0.0", [0_u8; 32])).unwrap();

        let installer = Installer::new(LocalFileDownloader);
        let error = block_on(installer.install(
            &manifest,
            &PluginNamespace::official(),
            ResolvedReleaseSource::universal(DownloadSource::Local(release_path), [0_u8; 32]),
            temp_dir.path(),
        ))
        .unwrap_err();

        match error {
            InstallError::Download(error) => match error.as_ref() {
                ora_utils::http::DownloadError::ChecksumMismatch { .. } => {}
                other => panic!("expected checksum mismatch, got {other:?}"),
            },
            other => panic!("expected checksum mismatch, got {other:?}"),
        }
        assert!(
            !temp_dir
                .path()
                .join("plugins")
                .join("installed")
                .join("official")
                .join("weather")
                .join("1.0.0")
                .exists()
        );
    }
    /// Imports a Skill archive that contains one or more required static Skill packages.
    #[test]
    fn imports_local_skill_archive_with_required_skill_assets() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("skill-pack.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    &b"identifier = \"ora.skill-pack\"\nnamespace = \"official\"\nkind = \"skill\"\nversion = \"0.1.1\"\ndescription = \"Skill plugin test\"\n"[..],
                ),
                (
                    "assets/review/SKILL.md",
                    b"---\nname: review\ndescription: Reviews code\n---\n".as_slice(),
                ),
                (
                    "assets/testing/SKILL.md",
                    b"---\nname: testing\ndescription: Tests code\n---\n".as_slice(),
                ),
                (
                    "assets/review/scripts/check.js",
                    b"export {};\n".as_slice(),
                ),
            ],
        );

        let installer = Installer::new(LocalFileDownloader);
        let package = installer
            .install_local(&release_path, temp_dir.path(), HostTarget::Unsupported)
            .expect("import static Skill archive");

        assert_eq!(
            package.id,
            installed_id(&PluginNamespace::local(), "ora.skill-pack"),
        );
        assert_eq!(
            package.package_dir,
            temp_dir
                .path()
                .join("plugins")
                .join("installed")
                .join("local")
                .join("ora.skill-pack")
                .join("0.1.1")
        );
        assert!(package.package_dir.join("orax.toml").is_file());
        assert!(
            package
                .package_dir
                .join("assets")
                .join("review")
                .join("SKILL.md")
                .is_file()
        );
        assert!(
            package
                .package_dir
                .join("assets")
                .join("testing")
                .join("SKILL.md")
                .is_file()
        );
        assert!(!package.package_dir.join("main.js").exists());
    }

    /// A Skill archive without `assets/<name>/SKILL.md` is never committed.
    #[test]
    fn rejects_local_skill_archive_without_skill_assets() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("incomplete-skill.orax");
        write_orax_zip(
            &release_path,
            &[(
                "orax.toml",
                &b"identifier = \"ora.skill-pack\"\nnamespace = \"official\"\nkind = \"skill\"\nversion = \"0.1.1\"\ndescription = \"Skill plugin test\"\n"[..],
            )],
        );

        let error = Installer::new(LocalFileDownloader)
            .install_local(&release_path, temp_dir.path(), HostTarget::Unsupported)
            .unwrap_err();

        assert!(matches!(
            error,
            InstallError::InvalidPackage { ref field_path, .. } if field_path == "skill"
        ));
        assert!(
            !temp_dir
                .path()
                .join("plugins")
                .join("installed")
                .join("local")
                .join("ora.skill-pack")
                .join("0.1.1")
                .exists()
        );
    }
    /// Imports a real-world agent archive into a brand-new profile whose `plugins/` root does
    /// not exist yet, proving an import is usable without a prior marketplace sync.
    #[test]
    fn imports_local_archive_into_a_fresh_profile() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("ora-space.opencode.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    &b"resolver = 1\nidentifier = \"ora-space.opencode\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"0.1.2\"\ndescription = \"Ora Space OpenCode Agent\"\n"[..],
                ),
                ("main.js", b"export {};\n".as_slice()),
                ("logo.svg", b"<svg/>".as_slice()),
            ],
        );

        let installer = Installer::new(LocalFileDownloader);
        let package = installer
            .install_local(&release_path, temp_dir.path(), HostTarget::Unsupported)
            .expect("import agent archive into a fresh profile");

        assert_eq!(
            package,
            InstalledPackage {
                package_dir: temp_dir
                    .path()
                    .join("plugins")
                    .join("installed")
                    .join("local")
                    .join("ora-space.opencode")
                    .join("0.1.2"),
                id: installed_id(&PluginNamespace::local(), "ora-space.opencode"),
            }
        );
        assert!(package.package_dir.join("main.js").exists());
        assert!(package.package_dir.join("logo.svg").exists());
    }

    /// Installs a targeted Hook release and verifies the package contains the executable and no
    /// main.js, and the artifact target matches the selected release target.
    #[test]
    fn installs_targeted_hook_release_with_executable() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("rtk.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK command rewrite hook\"\n\n[artifact]\ntarget = \"x86_64-pc-windows-msvc\"\n".as_slice(),
                ),
                (
                    "assets/config.json",
                    br#"{"schemaVersion":1,"hook":{"executable":"assets/rtk.exe","lifecycle":{"init":{"args":["--init"]}}}}"#.as_slice(),
                ),
                ("assets/rtk.exe", b"MZdummy".as_slice()),
            ],
        );
        let digest = sha256_file(&release_path);
        let manifest = PluginManifest::parse(&format!(
            "resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK command rewrite hook\"\n[[targets]]\ntarget = \"x86_64-pc-windows-msvc\"\nurl = \"https://example.com/rtk.orax\"\nsha256 = \"{}\"\n",
            hex(digest)
        ))
        .unwrap();

        let host_target = ora_plugin_manifest::HookTarget::parse("x86_64-pc-windows-msvc").unwrap();
        let source = ResolvedReleaseSource::targeted(
            DownloadSource::Local(release_path),
            digest,
            host_target,
        );
        let package_dir = block_on(Installer::new(LocalFileDownloader).install(
            &manifest,
            &PluginNamespace::official(),
            source,
            temp_dir.path(),
        ))
        .expect("install targeted hook release");

        assert!(package_dir.join("assets").join("rtk.exe").is_file());
        assert!(package_dir.join("assets").join("config.json").is_file());
        assert!(!package_dir.join("main.js").exists());
    }

    /// A targeted Hook archive whose artifact target differs from the selected release target
    /// is never committed as valid.
    #[test]
    fn rejects_targeted_hook_release_with_mismatched_artifact_target() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("rtk.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK command rewrite hook\"\n\n[artifact]\ntarget = \"aarch64-pc-windows-msvc\"\n".as_slice(),
                ),
                (
                    "assets/config.json",
                    br#"{"schemaVersion":1,"hook":{"executable":"assets/rtk.exe","lifecycle":{"init":{"args":["--init"]}}}}"#.as_slice(),
                ),
                ("assets/rtk.exe", b"MZdummy".as_slice()),
            ],
        );
        let digest = sha256_file(&release_path);
        let manifest = PluginManifest::parse(&format!(
            "resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK command rewrite hook\"\n[[targets]]\ntarget = \"x86_64-pc-windows-msvc\"\nurl = \"https://example.com/rtk.orax\"\nsha256 = \"{}\"\n",
            hex(digest)
        ))
        .unwrap();

        let host_target = ora_plugin_manifest::HookTarget::parse("x86_64-pc-windows-msvc").unwrap();
        let source = ResolvedReleaseSource::targeted(
            DownloadSource::Local(release_path),
            digest,
            host_target,
        );
        let error = block_on(Installer::new(LocalFileDownloader).install(
            &manifest,
            &PluginNamespace::official(),
            source,
            temp_dir.path(),
        ))
        .unwrap_err();

        assert!(matches!(error, InstallError::TargetMismatch { .. }));
        assert!(
            !temp_dir
                .path()
                .join("plugins")
                .join("installed")
                .join("official")
                .join("rtk-ai.rtk")
                .join("0.1.0")
                .exists()
        );
    }

    /// Builds one `.orax` whose named entries are stored with the Unix executable mode.
    fn write_orax_zip_with_executables(path: &Path, files: &[(&str, &[u8])], executables: &[&str]) {
        let mut writer = ZipWriter::new(File::create(path).unwrap());
        for (name, data) in files {
            let options = if executables.contains(name) {
                SimpleFileOptions::default().unix_permissions(0o100_755)
            } else {
                SimpleFileOptions::default()
            };
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }

    /// Builds an installed agent manifest that self-declares the target it was built for.
    fn bundled_agent_manifest(target: &str) -> String {
        format!(
            "resolver = 1\nidentifier = \"ora-space.opencode\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"0.2.4\"\ndescription = \"OpenCode agent\"\n\n[artifact]\ntarget = \"{target}\"\n"
        )
    }

    /// An agent package may ship the CLI it drives, landing it runnable and target-checked.
    ///
    /// This is the whole per-target agent release shape in one place: `[artifact]` naming the host
    /// it was built for, and an `assets/bin` executable whose execute bit has to survive extraction
    /// for the plugin to be able to spawn it at all.
    #[test]
    fn installs_a_local_agent_package_that_bundles_its_cli() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("opencode.orax");
        write_orax_zip_with_executables(
            &release_path,
            &[
                (
                    "orax.toml",
                    bundled_agent_manifest("x86_64-pc-windows-msvc").as_bytes(),
                ),
                ("main.js", b"export {};".as_slice()),
                ("assets/bin/opencode.exe", b"MZdummy".as_slice()),
            ],
            &["assets/bin/opencode.exe"],
        );

        let installed = Installer::new(LocalFileDownloader)
            .install_local(
                &release_path,
                temp_dir.path(),
                HostTarget::Triple(&windows_host()),
            )
            .unwrap();

        let executable = installed.package_dir.join("assets/bin/opencode.exe");
        assert!(executable.is_file(), "the bundled CLI must be installed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&executable).unwrap().permissions().mode() & 0o777,
                0o755,
                "a CLI that arrives without its execute bit can never be spawned"
            );
        }
    }

    /// An agent package built for another host is refused, exactly as a Hook package is.
    #[test]
    fn rejects_a_local_agent_package_built_for_another_host() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("opencode.orax");
        write_orax_zip_with_executables(
            &release_path,
            &[
                (
                    "orax.toml",
                    bundled_agent_manifest("aarch64-apple-darwin").as_bytes(),
                ),
                ("main.js", b"export {};".as_slice()),
                ("assets/bin/opencode", b"\x7fELFdummy".as_slice()),
            ],
            &["assets/bin/opencode"],
        );

        let error = Installer::new(LocalFileDownloader)
            .install_local(
                &release_path,
                temp_dir.path(),
                HostTarget::Triple(&windows_host()),
            )
            .unwrap_err();

        assert!(matches!(error, InstallError::TargetMismatch { .. }));
    }

    /// An agent that resolves its CLI from PATH bundles nothing and declares no target, so it
    /// installs on any host without the artifact check applying at all.
    #[test]
    fn installs_a_local_agent_package_without_a_bundled_cli() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("opencode.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"ora-space.opencode\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"0.2.4\"\ndescription = \"OpenCode agent\"\n".as_slice(),
                ),
                ("main.js", b"export {};".as_slice()),
            ],
        );

        let installed = Installer::new(LocalFileDownloader)
            .install_local(&release_path, temp_dir.path(), HostTarget::Unsupported)
            .unwrap();

        assert!(installed.package_dir.join("main.js").is_file());
    }

    /// Local import of a Hook archive whose `[artifact]` target does not match the host is
    /// rejected so a wrong-ABI package never lands as valid.
    #[test]
    fn rejects_local_hook_import_with_mismatched_host_target() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("rtk.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK command rewrite hook\"\n\n[artifact]\ntarget = \"aarch64-apple-darwin\"\n".as_slice(),
                ),
                (
                    "assets/config.json",
                    br#"{"schemaVersion":1,"hook":{"executable":"assets/rtk.exe","lifecycle":{"init":{"args":["--init"]}}}}"#.as_slice(),
                ),
                ("assets/rtk.exe", b"MZdummy".as_slice()),
            ],
        );

        let error = Installer::new(LocalFileDownloader)
            .install_local(
                &release_path,
                temp_dir.path(),
                HostTarget::Triple(&windows_host()),
            )
            .unwrap_err();

        assert!(matches!(error, InstallError::TargetMismatch { .. }));
    }

    /// A targeted marketplace install whose extracted package lacks `[artifact]` fails closed.
    #[test]
    fn rejects_targeted_hook_release_without_artifact_section() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("rtk.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK command rewrite hook\"\n".as_slice(),
                ),
                (
                    "assets/config.json",
                    br#"{"schemaVersion":1,"hook":{"executable":"assets/rtk.exe","lifecycle":{"init":{"args":["--init"]}}}}"#.as_slice(),
                ),
                ("assets/rtk.exe", b"MZdummy".as_slice()),
            ],
        );
        let digest = sha256_file(&release_path);
        let manifest = PluginManifest::parse(&format!(
            "resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK command rewrite hook\"\n[[targets]]\ntarget = \"x86_64-pc-windows-msvc\"\nurl = \"https://example.com/rtk.orax\"\nsha256 = \"{}\"\n",
            hex(digest)
        ))
        .unwrap();
        let source = ResolvedReleaseSource::targeted(
            DownloadSource::Local(release_path),
            digest,
            windows_host(),
        );
        let error = block_on(Installer::new(LocalFileDownloader).install(
            &manifest,
            &PluginNamespace::official(),
            source,
            temp_dir.path(),
        ))
        .unwrap_err();
        assert!(matches!(
            error,
            InstallError::MissingArtifactTarget | InstallError::InvalidManifest(_)
        ));
    }

    /// select_release returns NoArtifactForTarget when the host target has no matching artifact.
    #[test]
    fn select_release_rejects_unsupported_host_target() {
        let digest = format!("{}{}", "ab".repeat(31), "ab");
        let manifest = PluginManifest::parse(&format!(
            "resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK hook\"\n[[targets]]\ntarget = \"x86_64-pc-windows-msvc\"\nurl = \"https://example.com/rtk.orax\"\nsha256 = \"{digest}\"\n"
        ))
        .unwrap();
        let host_target = ora_plugin_manifest::HookTarget::parse("aarch64-apple-darwin").unwrap();
        let error = super::select_release(&manifest, HostTarget::Triple(&host_target)).unwrap_err();
        assert!(matches!(error, InstallError::NoArtifactForTarget { .. }));
    }

    /// A universal release can be selected without a host triple, so agent/MCP/skill packages
    /// stay installable on hosts that are not Hook targets.
    #[test]
    fn select_release_accepts_a_universal_release_without_a_host_target() {
        let digest = format!("{}{}", "ab".repeat(31), "ab");
        let manifest = PluginManifest::parse(&format!(
            "resolver = 1\nidentifier = \"weather\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Weather\"\nurl = \"https://example.com/weather.orax\"\nsha256 = \"{digest}\"\n"
        ))
        .unwrap();
        super::select_release(&manifest, HostTarget::Unsupported)
            .expect("universal release does not need a host");
    }

    /// A universal object-key listing resolves to an S3 download source rather than an HTTPS URL.
    #[test]
    fn select_release_maps_an_object_key_to_s3() {
        let digest = format!("{}{}", "ab".repeat(31), "ab");
        let manifest = PluginManifest::parse(&format!(
            "resolver = 1\nidentifier = \"weather\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Weather\"\nurl = \"ora-space.opencode-v0.1.3.orax\"\nsha256 = \"{digest}\"\n"
        ))
        .unwrap();
        let source = super::select_release(&manifest, HostTarget::Unsupported)
            .expect("object-key universal release");
        assert_eq!(
            source.download(),
            &DownloadSource::S3 {
                key: "ora-space.opencode-v0.1.3.orax".to_owned(),
            }
        );
    }

    /// An HTTPS listing still resolves to a URL download source.
    #[test]
    fn select_release_maps_https_to_a_url() {
        let digest = format!("{}{}", "ab".repeat(31), "ab");
        let manifest = PluginManifest::parse(&format!(
            "resolver = 1\nidentifier = \"weather\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"Weather\"\nurl = \"https://example.com/weather.orax\"\nsha256 = \"{digest}\"\n"
        ))
        .unwrap();
        let source = super::select_release(&manifest, HostTarget::Unsupported)
            .expect("https universal release");
        match source.download() {
            DownloadSource::Url(url) => {
                assert_eq!(url.as_str(), "https://example.com/weather.orax");
            }
            other => panic!("expected a URL source, got {other:?}"),
        }
    }

    /// A targeted release cannot be selected when the compiled host is not a plugin target.
    #[test]
    fn select_release_rejects_a_targeted_release_without_a_host_target() {
        let digest = format!("{}{}", "ab".repeat(31), "ab");
        let manifest = PluginManifest::parse(&format!(
            "resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK hook\"\n[[targets]]\ntarget = \"x86_64-pc-windows-msvc\"\nurl = \"https://example.com/rtk.orax\"\nsha256 = \"{digest}\"\n"
        ))
        .unwrap();
        let error = super::select_release(&manifest, HostTarget::Unsupported).unwrap_err();
        assert!(matches!(error, InstallError::UnsupportedHost));
    }

    /// Local Hook import without a host target fails closed rather than skipping the match.
    #[test]
    fn rejects_local_hook_import_without_a_host_target() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("rtk.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK command rewrite hook\"\n\n[artifact]\ntarget = \"x86_64-pc-windows-msvc\"\n".as_slice(),
                ),
                (
                    "assets/config.json",
                    br#"{"schemaVersion":1,"hook":{"executable":"assets/rtk.exe","lifecycle":{"init":{"args":["--init"]}}}}"#.as_slice(),
                ),
                ("assets/rtk.exe", b"MZdummy".as_slice()),
            ],
        );

        let error = Installer::new(LocalFileDownloader)
            .install_local(&release_path, temp_dir.path(), HostTarget::Unsupported)
            .unwrap_err();
        assert!(matches!(error, InstallError::UnsupportedHost));
    }

    /// Stages one installed package version below `<data>/plugins/installed/official/weather`.
    fn stage_installed_weather_version(data_dir: &Path, version: &str) {
        let package_root = data_dir
            .join("plugins")
            .join("installed")
            .join("official")
            .join("weather")
            .join(version);
        std::fs::create_dir_all(&package_root).unwrap();
        std::fs::write(
            package_root.join("orax.toml"),
            format!(
                "resolver = 1\nidentifier = \"weather\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"{version}\"\ndescription = \"A test plugin\"\n"
            ),
        )
        .unwrap();
        std::fs::write(package_root.join("main.js"), "export {};\n").unwrap();
    }

    /// Updates an installed plugin to the marketplace release and retires the older package.
    #[test]
    fn updates_installed_plugin_and_retires_old_versions() {
        let temp_dir = TempDir::new().unwrap();
        stage_installed_weather_version(temp_dir.path(), "0.9.0");
        let release_path = temp_dir.path().join("weather-1.0.0.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    &b"resolver = 1\nidentifier = \"weather\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"A test plugin\"\n"[..],
                ),
                ("main.js", b"export {};\n".as_slice()),
                ("logo.svg", b"<svg/>".as_slice()),
            ],
        );
        let digest = sha256_file(&release_path);
        let manifest =
            PluginManifest::parse(&manifest_with_digest("weather", "1.0.0", digest)).unwrap();

        let recorded_progress = Arc::new(Mutex::new(Vec::new()));
        let progress = {
            let recorded_progress = Arc::clone(&recorded_progress);
            Arc::new(move |snapshot| recorded_progress.lock().unwrap().push(snapshot))
        };
        let package = block_on(
            Installer::new(ProgressReportingLocalDownloader).update_with_progress(
                &manifest,
                &PluginNamespace::official(),
                ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
                temp_dir.path(),
                progress,
            ),
        )
        .unwrap();

        let new_package = temp_dir
            .path()
            .join("plugins")
            .join("installed")
            .join("official")
            .join("weather")
            .join("1.0.0");
        assert_eq!(
            package,
            InstalledPackage {
                package_dir: new_package.clone(),
                id: installed_id(&PluginNamespace::official(), "weather"),
            }
        );
        assert!(new_package.join("main.js").is_file());
        assert_eq!(
            *recorded_progress.lock().unwrap(),
            vec![Progress {
                bytes: 3,
                total: Some(4),
            }]
        );
        assert!(
            !temp_dir
                .path()
                .join("plugins")
                .join("installed")
                .join("official")
                .join("weather")
                .join("0.9.0")
                .exists()
        );
    }

    /// An update is refused when the marketplace still publishes the installed version.
    #[test]
    fn rejects_updating_a_plugin_already_at_the_latest_version() {
        let temp_dir = TempDir::new().unwrap();
        stage_installed_weather_version(temp_dir.path(), "1.0.0");
        let release_path = temp_dir.path().join("weather-1.0.0.orax");
        write_orax_zip(
            &release_path,
            &[(
                "orax.toml",
                &b"resolver = 1\nidentifier = \"weather\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"A test plugin\"\n"[..],
            )],
        );
        let digest = sha256_file(&release_path);
        let manifest =
            PluginManifest::parse(&manifest_with_digest("weather", "1.0.0", digest)).unwrap();

        let error = block_on(Installer::new(LocalFileDownloader).update(
            &manifest,
            &PluginNamespace::official(),
            ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            temp_dir.path(),
        ))
        .unwrap_err();

        assert!(matches!(
            error,
            UpdateError::AlreadyUpToDate { ref id, .. } if id == "official/weather"
        ));
        assert!(
            temp_dir
                .path()
                .join("plugins")
                .join("installed")
                .join("official")
                .join("weather")
                .join("1.0.0")
                .join("main.js")
                .is_file()
        );
    }

    /// The marketplace is never allowed to downgrade an installed plugin.
    #[test]
    fn rejects_downgrading_an_installed_plugin() {
        let temp_dir = TempDir::new().unwrap();
        stage_installed_weather_version(temp_dir.path(), "2.0.0");
        let release_path = temp_dir.path().join("weather-1.0.0.orax");
        write_orax_zip(
            &release_path,
            &[(
                "orax.toml",
                &b"resolver = 1\nidentifier = \"weather\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"A test plugin\"\n"[..],
            )],
        );
        let digest = sha256_file(&release_path);
        let manifest =
            PluginManifest::parse(&manifest_with_digest("weather", "1.0.0", digest)).unwrap();

        let error = block_on(Installer::new(LocalFileDownloader).update(
            &manifest,
            &PluginNamespace::official(),
            ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            temp_dir.path(),
        ))
        .unwrap_err();

        assert!(matches!(
            error,
            UpdateError::Downgrade { ref id, .. } if id == "official/weather"
        ));
        assert!(
            !temp_dir
                .path()
                .join("plugins")
                .join("installed")
                .join("official")
                .join("weather")
                .join("1.0.0")
                .exists()
        );
    }

    /// Updating a plugin that has no installed package reports NotFound.
    #[test]
    fn rejects_updating_a_plugin_that_is_not_installed() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("weather-1.0.0.orax");
        write_orax_zip(
            &release_path,
            &[(
                "orax.toml",
                &b"resolver = 1\nidentifier = \"weather\"\nnamespace = \"official\"\nkind = \"agent\"\nversion = \"1.0.0\"\ndescription = \"A test plugin\"\n"[..],
            )],
        );
        let digest = sha256_file(&release_path);
        let manifest =
            PluginManifest::parse(&manifest_with_digest("weather", "1.0.0", digest)).unwrap();

        let error = block_on(Installer::new(LocalFileDownloader).update(
            &manifest,
            &PluginNamespace::official(),
            ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            temp_dir.path(),
        ))
        .unwrap_err();

        assert!(matches!(
            error,
            UpdateError::NotFound { ref id } if id == "official/weather"
        ));
    }
    /// One valid Agent declaration whose bytes are the package's `assets/config.json`.
    const AGENT_DECLARATION: &[u8] = br#"{"schemaVersion":2,"agent":{"modelAccess":{"contractVersion":1,"protocols":["anthropic-messages"],"modelCardinality":"single"}}}"#;

    /// Builds an installed-form agent package whose declaration bytes are `declaration`.
    fn agent_package_files(declaration: Option<&[u8]>) -> Vec<(String, Vec<u8>)> {
        let mut files = vec![
            (
                "orax.toml".to_owned(),
                installed_agent_manifest("weather", "1.0.0").into_bytes(),
            ),
            ("main.js".to_owned(), b"export {};\n".to_vec()),
        ];
        if let Some(declaration) = declaration {
            files.push(("assets/config.json".to_owned(), declaration.to_vec()));
        }
        files
    }

    /// Renders the package files in the shape `write_orax_zip` takes.
    fn files_as_refs(files: &[(String, Vec<u8>)]) -> Vec<(&str, &[u8])> {
        files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect()
    }

    /// Installs one agent release whose listing declares the declaration association.
    ///
    /// The association binds `declaration`'s real digest, so every failure the caller observes
    /// comes from the package content rather than from the listing lying about its own release.
    fn install_declared_agent_release(
        data_dir: &Path,
        package_files: &[(String, Vec<u8>)],
    ) -> Result<PathBuf, InstallError> {
        let release_path = data_dir.join("pkg.orax");
        write_orax_zip(&release_path, &files_as_refs(package_files));
        let digest = sha256_file(&release_path);
        let declaration_digest = {
            let mut hasher = Sha256::new();
            hasher.update(AGENT_DECLARATION);
            hex(hasher.finalize().into())
        };
        let manifest = PluginManifest::parse(&format!(
            "{}\n[agent_configuration]\nschema_version = 2\ndeclaration_sha256 = \"{declaration_digest}\"\ncopy = \"agent-configuration.json\"\n",
            manifest_with_digest("weather", "1.0.0", digest)
        ))
        .unwrap();
        assert!(
            manifest.agent_configuration().is_some(),
            "the test listing must carry the association"
        );
        block_on(Installer::new(LocalFileDownloader).install(
            &manifest,
            &PluginNamespace::official(),
            ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            data_dir,
        ))
    }

    /// A listing that declares the association installs a package carrying exactly those
    /// declaration bytes, and the declaration survives into the installed package.
    #[test]
    fn installs_package_matching_the_declared_association() {
        let temp_dir = TempDir::new().unwrap();
        let package_dir = install_declared_agent_release(
            temp_dir.path(),
            &agent_package_files(Some(AGENT_DECLARATION)),
        )
        .unwrap();
        assert_eq!(
            fs::read(package_dir.join("assets").join("config.json")).unwrap(),
            AGENT_DECLARATION,
            "the verified declaration bytes land in the installed package"
        );
    }

    /// A package whose declaration bytes differ from the associated digest is rejected while it
    /// is still staged: a reformatted but semantically identical declaration is still a different
    /// release identity, because fingerprints cover the exact bytes.
    #[test]
    fn rejects_package_whose_declaration_bytes_differ() {
        let temp_dir = TempDir::new().unwrap();
        let reformatted = br#"{
  "schemaVersion": 2,
  "agent": {
    "modelAccess": {
      "contractVersion": 1,
      "protocols": ["anthropic-messages"],
      "modelCardinality": "single"
    }
  }
}"#;
        let error = install_declared_agent_release(
            temp_dir.path(),
            &agent_package_files(Some(reformatted)),
        )
        .unwrap_err();
        assert!(
            matches!(&error, InstallError::InvalidPackage { field_path, .. } if field_path == "assets/config.json"),
            "the rejection names the declaration file: {error:?}"
        );
        assert_no_install_residue(temp_dir.path(), "official", "weather", "1.0.0");
    }

    /// A package with no declaration at all cannot satisfy a listing that declares one: the
    /// association would bind a schema no package carries.
    #[test]
    fn rejects_package_without_a_declaration_when_the_listing_declares_one() {
        let temp_dir = TempDir::new().unwrap();
        let error = install_declared_agent_release(temp_dir.path(), &agent_package_files(None))
            .unwrap_err();
        assert!(
            matches!(&error, InstallError::InvalidPackage { field_path, .. } if field_path == "assets/config.json"),
            "the rejection names the declaration file: {error:?}"
        );
        assert_no_install_residue(temp_dir.path(), "official", "weather", "1.0.0");
    }

    /// A package may carry an Agent declaration while its listing declares no association: the
    /// declaration is inert package metadata until a release binds it, so the package installs.
    #[test]
    fn installs_declaring_package_from_a_listing_without_an_association() {
        let temp_dir = TempDir::new().unwrap();
        let files = agent_package_files(Some(AGENT_DECLARATION));
        let package_dir =
            install_weather_release(temp_dir.path(), "agent", &files_as_refs(&files)).unwrap();
        assert!(package_dir.join("assets").join("config.json").exists());
    }
    /// A non-agent kind cannot smuggle a personal-model declaration: the Agent shape is
    /// exclusive to agent packages, whatever its listing claims.
    #[test]
    fn rejects_an_agent_declaration_in_a_non_agent_package() {
        let temp_dir = TempDir::new().unwrap();
        let mut files = agent_package_files(Some(AGENT_DECLARATION));
        files[0] = (
            "orax.toml".to_owned(),
            b"resolver = 1\nidentifier = \"weather\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"A test plugin\"\n".to_vec(),
        );
        let error =
            install_weather_release(temp_dir.path(), "mcp", &files_as_refs(&files)).unwrap_err();
        assert!(
            matches!(&error, InstallError::InvalidPackage { field_path, message, .. }
                if field_path == "assets/config.json" && message.contains("Agent configuration")),
            "the rejection names the kind mismatch: {error:?}"
        );
        assert_no_install_residue(temp_dir.path(), "official", "weather", "1.0.0");
    }
}
