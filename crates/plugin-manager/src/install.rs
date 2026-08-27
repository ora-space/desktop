//! Installs a plugin release by downloading its package and safely extracting it.

use crate::discovery::installed_root;
use ora_plugin_manifest::{HookTarget, PluginKind, PluginManifest, PluginReleaseSource};
use ora_utils::archive::{ArchiveFormat, ExtractLimits, extract_archive};
use ora_utils::hash;
use ora_utils::http::{Checksum, DownloadOptions, DownloadRequest, DownloadSource, HttpDownload};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Sub-directory of a plugin data directory that caches downloaded release archives.
const CACHE_ROOT: &str = "cache";
/// Extension appended to a downloaded release archive filename.
const RELEASE_EXTENSION: &str = ".orax";

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
    /// The imported archive does not contain an `orax.toml` manifest at its root.
    #[error("imported archive does not contain orax.toml at its root")]
    MissingManifest,
    /// The in-archive `orax.toml` could not be parsed or validated.
    #[error("imported plugin manifest is invalid: {0}")]
    InvalidManifest(#[from] ora_plugin_manifest::ManifestError),
    /// The extracted package does not satisfy the host-side requirements for its kind.
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
    fn invalid_package(source: crate::validation::ManifestValidationError) -> Self {
        Self::InvalidPackage {
            field_path: source.field_path().to_owned(),
            message: source.to_string(),
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

/// Selects the release source matching `host_target` from a manifest's `release_source()`.
///
/// A universal release resolves to the single URL/digest pair regardless of host. A targeted
/// release requires a supported host and resolves to the one exact-matching target, returning
/// `UnsupportedHost` when the compiled host is not a plugin target and `NoArtifactForTarget`
/// when that host has no matching artifact, so the caller can reject before download.
pub fn select_release(
    manifest: &PluginManifest,
    host_target: Option<&HookTarget>,
) -> Result<ResolvedReleaseSource, InstallError> {
    match manifest.release_source() {
        Some(PluginReleaseSource::Universal { url, sha256 }) => {
            Ok(ResolvedReleaseSource::universal(
                DownloadSource::Url(url.as_url().clone()),
                *sha256.as_bytes(),
            ))
        }
        Some(PluginReleaseSource::Targets(targets)) => {
            let host_target = host_target.ok_or(InstallError::UnsupportedHost)?;
            let entry = targets
                .iter()
                .find(|entry| entry.target() == host_target)
                .ok_or_else(|| InstallError::NoArtifactForTarget {
                    target: host_target.to_string(),
                })?;
            Ok(ResolvedReleaseSource::targeted(
                DownloadSource::Url(entry.url().as_url().clone()),
                *entry.sha256().as_bytes(),
                entry.target().clone(),
            ))
        }
        None => Err(InstallError::MissingRelease),
    }
}

/// Describes one package materialized from a local release archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    /// The package directory below `<data-dir>/plugins/installed/<namespace>/<name>/<version>`.
    pub package_dir: PathBuf,
    /// The plugin identifier (`namespace/name`) derived from the in-archive manifest.
    pub id: String,
}

/// Orchestrates one plugin installation and stays backend-agnostic.
///
/// The downloader is injected so the orchestration never names a concrete transport; production
/// wiring supplies a network downloader while tests (and offline installs) use the local one.
#[derive(Clone)]
pub struct Installer<D> {
    downloader: D,
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
    /// For a universal release, `source` carries the download URL and the manifest's `sha256`
    /// verifies the bytes. For a targeted release, `source` is the resolved target-specific
    /// artifact (selected by the caller against `host_target`) and the matching target digest
    /// verifies the bytes; the installer then confirms the extracted package's artifact target
    /// equals the selected release target so a wrong-architecture archive never installs as valid.
    pub async fn install(
        &self,
        manifest: &PluginManifest,
        source: ResolvedReleaseSource,
        data_dir: &Path,
    ) -> Result<PathBuf, InstallError> {
        let archive_path = self
            .download_package(manifest, source.clone(), data_dir)
            .await?;
        let namespace = manifest.namespace();
        let name = manifest.name();
        let version = manifest.version().to_string();
        let package_parent = installed_root(data_dir)
            .join(namespace.as_str())
            .join(name.as_str());
        let package_dir = package_parent.join(&version);
        if package_dir.exists() {
            return Err(InstallError::AlreadyInstalled {
                path: package_dir,
                namespace: namespace.as_str().to_owned(),
                name: name.as_str().to_owned(),
                version,
            });
        }
        std::fs::create_dir_all(&package_parent).map_err(|source| InstallError::Io {
            path: package_parent.clone(),
            source,
        })?;
        let staging = tempfile::tempdir_in(&package_parent).map_err(|source| InstallError::Io {
            path: package_parent,
            source,
        })?;
        extract_archive(
            ArchiveFormat::Zip,
            &archive_path,
            staging.path(),
            &ExtractLimits::default(),
        )
        .map_err(|source| InstallError::Extract {
            path: staging.path().to_path_buf(),
            source,
        })?;
        crate::validation::validate(staging.path(), manifest, None)
            .map_err(InstallError::invalid_package)?;
        // A targeted archive must self-declare its target in an in-package `[artifact]` section.
        // Missing the installed manifest or the section fails closed so a wrong-architecture
        // archive cannot install as valid. Local import applies the same check against the host.
        if let Some(selected) = source.target() {
            let installed_manifest_path = staging.path().join(crate::discovery::MANIFEST_FILE_NAME);
            if !installed_manifest_path.is_file() {
                return Err(InstallError::MissingManifest);
            }
            let installed_source =
                std::fs::read_to_string(&installed_manifest_path).map_err(|source| {
                    InstallError::Io {
                        path: installed_manifest_path.clone(),
                        source,
                    }
                })?;
            let installed_manifest =
                ora_plugin_manifest::PluginManifest::parse_installed(&installed_source)?;
            let Some(artifact) = installed_manifest.artifact() else {
                return Err(InstallError::MissingArtifactTarget);
            };
            if selected != artifact.target() {
                return Err(InstallError::TargetMismatch {
                    release: selected.to_string(),
                    artifact: artifact.target().to_string(),
                });
            }
        }
        std::fs::rename(staging.path(), &package_dir).map_err(|source| InstallError::Io {
            path: package_dir.clone(),
            source,
        })?;
        Ok(package_dir)
    }

    /// Imports an already-downloaded release archive from `archive_path` into the installed tree.
    ///
    /// Unlike a marketplace install, the manifest lives inside the archive, so this extracts into
    /// a disposable staging directory first, reads and validates the in-archive `orax.toml`, and
    /// only then moves the verified tree into
    /// `<data-dir>/plugins/installed/<namespace>/<name>/<version>`. A `sha256` declared by the
    /// in-archive manifest is checked against the archive before anything is committed. A Hook
    /// archive must self-declare `[artifact]` and that target must match `host_target`; other
    /// kinds ignore the host. `host_target` is `None` when the compiled host is not a supported
    /// plugin target, which refuses Hook imports and leaves universal packages installable.
    pub fn install_local(
        &self,
        archive_path: &Path,
        data_dir: &Path,
        host_target: Option<&HookTarget>,
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
            &ExtractLimits::default(),
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

        let namespace = manifest.namespace();
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
        crate::validation::validate(staging.path(), &manifest, None)
            .map_err(InstallError::invalid_package)?;
        if matches!(manifest.kind(), PluginKind::Hook) {
            let Some(artifact) = manifest.artifact() else {
                return Err(InstallError::MissingArtifactTarget);
            };
            let Some(host_target) = host_target else {
                return Err(InstallError::UnsupportedHost);
            };
            if artifact.target() != host_target {
                return Err(InstallError::TargetMismatch {
                    release: host_target.to_string(),
                    artifact: artifact.target().to_string(),
                });
            }
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
            id: format!("{}/{}", namespace.as_str(), name.as_str()),
        })
    }

    /// Fetches and verifies one release archive into the cache, returning its path.
    async fn download_package(
        &self,
        manifest: &PluginManifest,
        source: ResolvedReleaseSource,
        data_dir: &Path,
    ) -> Result<PathBuf, InstallError> {
        let digest = source.sha256();
        let cache_dir = data_dir.join("plugins").join(CACHE_ROOT);
        std::fs::create_dir_all(&cache_dir).map_err(|error| InstallError::Io {
            path: cache_dir.clone(),
            source: error,
        })?;
        let archive_name = format!(
            "{}-{}{}",
            manifest.name().as_str(),
            manifest.version(),
            RELEASE_EXTENSION
        );
        let archive_path = cache_dir.join(archive_name);
        let request = DownloadRequest {
            source: source.download().clone(),
            destination: archive_path.clone(),
            checksum: Some(Checksum::sha256(digest.to_vec())),
            options: DownloadOptions::default(),
            progress: None,
            cancel: None,
        };
        self.downloader.download(request).await?;
        Ok(archive_path)
    }
}

#[cfg(test)]
mod tests {
    use super::{InstallError, InstalledPackage, Installer, ResolvedReleaseSource};
    use futures::executor::block_on;
    use ora_plugin_manifest::{HookTarget, PluginManifest};
    use ora_utils::http::{DownloadSource, LocalFileDownloader};
    use pretty_assertions::assert_eq;
    use sha2::{Digest, Sha256};
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;
    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

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
            "resolver = 1\nidentifier = \"{name}\"\nnamespace = \"official\"\nkind = \"{kind}\"\nversion = \"{version}\"\ndescription = \"A test plugin\"\nurl = \"https://example.com/{name}.orax\"\nsha256 = \"{}\"\n",
            hex(digest)
        )
    }

    /// Verifies a full local install: cache download, checksum, and safe extraction.
    #[test]
    fn installs_local_release_end_to_end() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("pkg.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"weather\"\n".as_slice(),
                ),
                ("main.js", b"export {};\n".as_slice()),
                ("logo.svg", b"<svg/>".as_slice()),
            ],
        );
        let manifest = PluginManifest::parse(&manifest_with_digest(
            "weather",
            "1.0.0",
            sha256_file(&release_path),
        ))
        .unwrap();

        let installer = Installer::new(LocalFileDownloader);
        let digest = sha256_file(&release_path);
        let package_dir = block_on(installer.install(
            &manifest,
            ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            temp_dir.path(),
        ))
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
            temp_dir
                .path()
                .join("plugins")
                .join("cache")
                .join("weather-1.0.0.orax")
                .exists()
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
            .install_local(&release_path, temp_dir.path(), None)
            .expect("import static Skill archive");

        assert_eq!(package.id, "official/ora.skill-pack");
        assert_eq!(
            package.package_dir,
            temp_dir
                .path()
                .join("plugins")
                .join("installed")
                .join("official")
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
            .install_local(&release_path, temp_dir.path(), None)
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
                .join("official")
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
            .install_local(&release_path, temp_dir.path(), None)
            .expect("import agent archive into a fresh profile");

        assert_eq!(
            package,
            InstalledPackage {
                package_dir: temp_dir
                    .path()
                    .join("plugins")
                    .join("installed")
                    .join("official")
                    .join("ora-space.opencode")
                    .join("0.1.2"),
                id: "official/ora-space.opencode".to_owned(),
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
                    br#"{"schemaVersion":1,"hook":{"protocol":"rtk-rewrite-v1","executable":"assets/rtk.exe","command":"rtk","toolVersion":"0.45.0"}}"#.as_slice(),
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
                    br#"{"schemaVersion":1,"hook":{"protocol":"rtk-rewrite-v1","executable":"assets/rtk.exe","command":"rtk","toolVersion":"0.45.0"}}"#.as_slice(),
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
                    br#"{"schemaVersion":1,"hook":{"protocol":"rtk-rewrite-v1","executable":"assets/rtk.exe","command":"rtk","toolVersion":"0.45.0"}}"#.as_slice(),
                ),
                ("assets/rtk.exe", b"MZdummy".as_slice()),
            ],
        );

        let error = Installer::new(LocalFileDownloader)
            .install_local(&release_path, temp_dir.path(), Some(&windows_host()))
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
                    br#"{"schemaVersion":1,"hook":{"protocol":"rtk-rewrite-v1","executable":"assets/rtk.exe","command":"rtk","toolVersion":"0.45.0"}}"#.as_slice(),
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
        let error = super::select_release(&manifest, Some(&host_target)).unwrap_err();
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
        super::select_release(&manifest, None).expect("universal release does not need a host");
    }

    /// A targeted release cannot be selected when the compiled host is not a plugin target.
    #[test]
    fn select_release_rejects_a_targeted_release_without_a_host_target() {
        let digest = format!("{}{}", "ab".repeat(31), "ab");
        let manifest = PluginManifest::parse(&format!(
            "resolver = 1\nidentifier = \"rtk-ai.rtk\"\nnamespace = \"official\"\nkind = \"hook\"\nversion = \"0.1.0\"\ndescription = \"RTK hook\"\n[[targets]]\ntarget = \"x86_64-pc-windows-msvc\"\nurl = \"https://example.com/rtk.orax\"\nsha256 = \"{digest}\"\n"
        ))
        .unwrap();
        let error = super::select_release(&manifest, None).unwrap_err();
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
                    br#"{"schemaVersion":1,"hook":{"protocol":"rtk-rewrite-v1","executable":"assets/rtk.exe","command":"rtk","toolVersion":"0.45.0"}}"#.as_slice(),
                ),
                ("assets/rtk.exe", b"MZdummy".as_slice()),
            ],
        );

        let error = Installer::new(LocalFileDownloader)
            .install_local(&release_path, temp_dir.path(), None)
            .unwrap_err();
        assert!(matches!(error, InstallError::UnsupportedHost));
    }
}
