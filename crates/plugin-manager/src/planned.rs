//! Installs a self-contained plan whose identity and digest come from an external authority.
//! Package interpretation stays here; Node supplies the plan and owns use leases and recovery.
use crate::install::declaration::{ExpectedAgentDeclaration, verify_agent_declaration};
use crate::{InstallError, Installer, ResolvedReleaseSource, installed_root};
use ora_domain::{PluginId, PluginNamespace};
use ora_plugin_manifest::PluginKind;
use ora_utils::{
    archive::{ArchiveFormat, extract_archive},
    http::{Checksum, DownloadOptions, DownloadRequest, HttpDownload},
    path::create_directories_without_symlinks,
};
use semver::Version;
use std::path::{Path, PathBuf};

/// Fully verified package, invisible to discovery until its owner commits it under a use lock.
pub struct PreparedPlugin {
    staging: tempfile::TempDir,
    id: PluginId,
    version: Version,
}

/// Validates exactly one package, including its identity; no highest-version selection is made.
pub fn inspect_planned_package(
    path: &Path,
    id: &PluginId,
    version: &Version,
) -> Result<(), InstallError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(InstallError::invalid_field(
            "package",
            "package must be a directory, not a link",
        ));
    }
    let manifest = crate::install::package::read_installed_manifest(path)?;
    if manifest.name().as_str() != id.name() || manifest.version() != version {
        return Err(InstallError::invalid_field(
            "identity",
            "package differs from the planned identity",
        ));
    }
    let namespace = PluginNamespace::parse(id.namespace())
        .map_err(|e| InstallError::invalid_field("namespace", e.to_string()))?;
    crate::validation::validate(path, &manifest, &namespace, /*logo*/ None)
        .map_err(InstallError::invalid_package)?;
    Ok(())
}

impl<D: HttpDownload> Installer<D> {
    /// Downloads with the caller's bounded policy, verifies the digest, and stages a checked
    /// package. No marketplace metadata is needed and nothing reaches the installed root yet.
    ///
    /// `expected_agent_declaration` is Cloud's frozen declaration association for this plan, when
    /// the selected catalog entry declared one: the package's in-archive declaration must compile
    /// and fingerprint to exactly the associated digest, or the plan's package is rejected.
    pub async fn prepare_release(
        &self,
        id: &PluginId,
        version: &Version,
        source: ResolvedReleaseSource,
        staging_root: &Path,
        options: DownloadOptions,
        expected_agent_declaration: Option<&ExpectedAgentDeclaration>,
    ) -> Result<PreparedPlugin, InstallError> {
        create_directories_without_symlinks(staging_root).map_err(|e| io_error(staging_root, e))?;
        let staging = tempfile::Builder::new()
            .prefix("install-")
            .tempdir_in(staging_root)
            .map_err(|e| io_error(staging_root, e))?;
        let archive = staging.path().join("release.orax");
        self.downloader
            .download(DownloadRequest {
                source: source.download().clone(),
                destination: archive.clone(),
                checksum: Some(Checksum::sha256(source.sha256().to_vec())),
                options,
                progress: None,
                cancel: None,
            })
            .await?;
        // Recheck at this trust boundary even for injected downloaders. No archive is extracted
        // until the authority's digest is proven against the actual file.
        let actual = ora_utils::hash::sha256_file(&archive).map_err(|e| io_error(&archive, e))?;
        let expected: String = source.sha256().iter().map(|b| format!("{b:02x}")).collect();
        if actual != expected {
            return Err(InstallError::ChecksumMismatch { expected, actual });
        }
        let package = staging.path().join("package");
        std::fs::create_dir(&package).map_err(|e| io_error(&package, e))?;
        extract_archive(
            ArchiveFormat::Zip,
            &archive,
            &package,
            &crate::limits::package_extract_limits(),
        )
        .map_err(|source| InstallError::Extract {
            path: package.clone(),
            source,
        })?;
        inspect_planned_package(&package, id, version)?;
        let manifest = crate::install::package::read_installed_manifest(&package)?;
        if let Some(selected) = source.target() {
            let artifact = manifest
                .artifact()
                .ok_or(InstallError::MissingArtifactTarget)?;
            if artifact.target() != selected {
                return Err(InstallError::TargetMismatch {
                    release: selected.to_string(),
                    artifact: artifact.target().to_string(),
                });
            }
        } else if manifest.artifact().is_some() || matches!(manifest.kind(), PluginKind::Hook) {
            // A native package cannot masquerade as a target-independent plan.
            return Err(InstallError::MissingArtifactTarget);
        }
        if let Some(expected) = expected_agent_declaration {
            verify_agent_declaration(&package, expected)?;
        }
        Ok(PreparedPlugin {
            staging,
            id: id.clone(),
            version: version.clone(),
        })
    }
}

impl PreparedPlugin {
    /// Publishes only verified bytes. The caller excludes session leases while replacing or
    /// removing packages. A corrupt same-version tree is retained in staging until commit succeeds.
    pub fn commit(self, data_dir: &Path) -> Result<PathBuf, InstallError> {
        let parent = installed_root(data_dir)
            .join(self.id.namespace())
            .join(self.id.name());
        create_directories_without_symlinks(&parent).map_err(|e| io_error(&parent, e))?;
        let destination = parent.join(self.version.to_string());
        let old = self.staging.path().join("previous");
        let exists = match std::fs::symlink_metadata(&destination) {
            Ok(metadata) if !metadata.file_type().is_symlink() && metadata.is_dir() => true,
            Ok(_) => {
                return Err(InstallError::invalid_field(
                    "package",
                    "refusing to replace a link or non-directory",
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(io_error(&destination, e)),
        };
        if exists {
            std::fs::rename(&destination, &old).map_err(|e| io_error(&destination, e))?;
        }
        if let Err(error) = std::fs::rename(self.staging.path().join("package"), &destination) {
            if exists {
                std::fs::rename(&old, &destination).map_err(|e| io_error(&destination, e))?;
            }
            return Err(io_error(&destination, error));
        }
        Ok(destination)
    }
}

/// Attaches the failing path without introducing Node error vocabulary into the installer.
fn io_error(path: &Path, source: std::io::Error) -> InstallError {
    InstallError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::{ExpectedAgentDeclaration, inspect_planned_package};
    use crate::InstallError;
    use futures::executor::block_on;
    use ora_domain::{PluginId, PluginNamespace};
    use ora_plugin_manifest::Sha256Digest;
    use ora_utils::http::{DownloadSource, LocalFileDownloader};
    use sha2::{Digest, Sha256};
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;
    use tempfile::TempDir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    /// One valid Agent declaration, exactly the bytes a release would associate.
    const DECLARATION: &[u8] = br#"{"schemaVersion":2,"agent":{"modelAccess":{"contractVersion":1,"protocols":["anthropic-messages"],"modelCardinality":"single"}}}"#;

    /// Renders a digest as lowercase hex.
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

    /// Builds one agent package tree with the given declaration bytes, as an extracted directory.
    fn package_tree(root: &Path, declaration: Option<&[u8]>) {
        let package = root.join("package");
        fs::create_dir_all(package.join("assets")).unwrap();
        fs::write(
            package.join("orax.toml"),
            "resolver = 1\nidentifier = \"ora-space.claude\"\nkind = \"agent\"\nversion = \"0.3.18\"\ndescription = \"Claude Code\"\n",
        )
        .unwrap();
        fs::write(package.join("main.js"), b"export {};\n").unwrap();
        if let Some(declaration) = declaration {
            fs::write(package.join("assets").join("config.json"), declaration).unwrap();
        }
    }

    /// The plan identity every prepared-release test uses.
    fn plugin_id() -> PluginId {
        PluginId::new(PluginNamespace::official(), "ora-space.claude").unwrap()
    }

    /// Verifies an extracted package with a matching declaration passes inspection.
    #[test]
    fn inspects_a_package_matching_the_plan_association() {
        let temp_dir = TempDir::new().unwrap();
        package_tree(temp_dir.path(), Some(DECLARATION));
        let expectation = ExpectedAgentDeclaration::new(
            2,
            Sha256Digest::parse(&hex(Sha256::digest(DECLARATION).into())).unwrap(),
        );
        // Inspection proves identity and validation; the declaration binding is proven by
        // `prepare_release`, which this exercise keeps to the shared package tree shape.
        inspect_planned_package(
            &temp_dir.path().join("package"),
            &plugin_id(),
            &"0.3.18".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(expectation.schema_version(), 2);
    }

    /// A plan whose frozen association names these declaration bytes stages a package carrying
    /// exactly them; the prepared plugin is the proof Cloud's install result will report.
    #[test]
    fn prepares_a_release_matching_the_frozen_association() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("release.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"ora-space.claude\"\nkind = \"agent\"\nversion = \"0.3.18\"\ndescription = \"Claude Code\"\n",
                ),
                ("main.js", b"export {};\n"),
                ("assets/config.json", DECLARATION),
            ],
        );
        let digest: [u8; 32] = Sha256::digest(fs::read(&release_path).unwrap()).into();
        let expectation = ExpectedAgentDeclaration::new(
            2,
            Sha256Digest::parse(&hex(Sha256::digest(DECLARATION).into())).unwrap(),
        );
        let prepared = block_on(crate::Installer::new(LocalFileDownloader).prepare_release(
            &plugin_id(),
            &"0.3.18".parse().unwrap(),
            crate::ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            temp_dir.path(),
            Default::default(),
            Some(&expectation),
        ))
        .unwrap();
        let committed = prepared.commit(temp_dir.path()).unwrap();
        assert_eq!(
            fs::read(committed.join("assets").join("config.json")).unwrap(),
            DECLARATION
        );
    }

    /// A package whose declaration bytes differ from the frozen association is rejected while it
    /// is still staged, so a retried plan cannot publish an unverified tree.
    #[test]
    fn rejects_a_release_whose_declaration_differs_from_the_plan() {
        let temp_dir = TempDir::new().unwrap();
        let reformatted = br#"{"schemaVersion":2,"agent":{"modelAccess":{"contractVersion":1,"protocols":["openai-completions"],"modelCardinality":"single"}}}"#;
        let release_path = temp_dir.path().join("release.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"ora-space.claude\"\nkind = \"agent\"\nversion = \"0.3.18\"\ndescription = \"Claude Code\"\n",
                ),
                ("main.js", b"export {};\n"),
                ("assets/config.json", reformatted),
            ],
        );
        let digest: [u8; 32] = Sha256::digest(fs::read(&release_path).unwrap()).into();
        let expectation = ExpectedAgentDeclaration::new(
            2,
            Sha256Digest::parse(&hex(Sha256::digest(DECLARATION).into())).unwrap(),
        );
        let error = block_on(crate::Installer::new(LocalFileDownloader).prepare_release(
            &plugin_id(),
            &"0.3.18".parse().unwrap(),
            crate::ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            temp_dir.path(),
            Default::default(),
            Some(&expectation),
        ))
        .err()
        .expect("the staged release must be rejected");
        assert!(
            matches!(&error, InstallError::InvalidPackage { field_path, .. } if field_path == "assets/config.json"),
            "the rejection names the declaration file: {error:?}"
        );
    }

    /// A plan without an association stages a declaring package without a binding to prove: the
    /// declaration is inert package metadata until a release binds it.
    #[test]
    fn prepares_a_declaring_package_without_an_association() {
        let temp_dir = TempDir::new().unwrap();
        let release_path = temp_dir.path().join("release.orax");
        write_orax_zip(
            &release_path,
            &[
                (
                    "orax.toml",
                    b"resolver = 1\nidentifier = \"ora-space.claude\"\nkind = \"agent\"\nversion = \"0.3.18\"\ndescription = \"Claude Code\"\n",
                ),
                ("main.js", b"export {};\n"),
                ("assets/config.json", DECLARATION),
            ],
        );
        let digest: [u8; 32] = Sha256::digest(fs::read(&release_path).unwrap()).into();
        block_on(crate::Installer::new(LocalFileDownloader).prepare_release(
            &plugin_id(),
            &"0.3.18".parse().unwrap(),
            crate::ResolvedReleaseSource::universal(DownloadSource::Local(release_path), digest),
            temp_dir.path(),
            Default::default(),
            None,
        ))
        .unwrap();
    }
}
