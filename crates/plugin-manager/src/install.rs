//! Installs a plugin release by downloading its package and safely extracting it.

use crate::discovery::installed_root;
use ora_plugin_manifest::PluginManifest;
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

    /// Downloads `manifest`'s release from `source` into the cache, verifies its digest, and
    /// extracts it into `<data-dir>/plugins/installed/<namespace>/<name>/<version>`, returning that
    /// package directory.
    ///
    /// Callers pass `DownloadSource::Url(manifest.url().as_url().clone())` for online installs,
    /// or a `Local` path for offline and test installs; either way the manifest's `sha256` is
    /// enforced during the download and only a verified package ever reaches the extraction step.
    pub async fn install(
        &self,
        manifest: &PluginManifest,
        source: DownloadSource,
        data_dir: &Path,
    ) -> Result<PathBuf, InstallError> {
        let archive_path = self.download_package(manifest, source, data_dir).await?;
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
    /// in-archive manifest is checked against the archive before anything is committed.
    pub fn install_local(
        &self,
        archive_path: &Path,
        data_dir: &Path,
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
        source: DownloadSource,
        data_dir: &Path,
    ) -> Result<PathBuf, InstallError> {
        let digest = manifest.sha256().ok_or(InstallError::MissingRelease)?;
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
            source,
            destination: archive_path.clone(),
            checksum: Some(Checksum::sha256(digest.as_bytes().to_vec())),
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
    use super::{InstallError, InstalledPackage, Installer};
    use futures::executor::block_on;
    use ora_plugin_manifest::PluginManifest;
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
        let package_dir = block_on(installer.install(
            &manifest,
            DownloadSource::Local(release_path),
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

        let package_dir = block_on(Installer::new(LocalFileDownloader).install(
            &manifest,
            DownloadSource::Local(release_path),
            temp_dir.path(),
        ))
        .expect("install marketplace Skill release");

        assert!(package_dir.join("assets/review/SKILL.md").is_file());
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
            DownloadSource::Local(release_path),
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
            .install_local(&release_path, temp_dir.path())
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
        assert!(package.package_dir.join("assets/review/SKILL.md").is_file());
        assert!(
            package
                .package_dir
                .join("assets/testing/SKILL.md")
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
            .install_local(&release_path, temp_dir.path())
            .unwrap_err();

        assert!(matches!(
            error,
            InstallError::InvalidPackage { ref field_path, .. } if field_path == "skill"
        ));
        assert!(
            !temp_dir
                .path()
                .join("plugins/installed/official/ora.skill-pack/0.1.1")
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
            .install_local(&release_path, temp_dir.path())
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
}
