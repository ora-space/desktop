//! Turns one verified release archive into a committed installed package.
//!
//! Everything a package must satisfy is checked here, while the extracted tree is still staged:
//! the manifest *inside* the archive is parsed and validated with the same code path discovery
//! uses, its identity is compared against the listing that published it, and a targeted release's
//! self-declared artifact target is matched against the selected release target. The caller
//! deletes the archive and any empty package parent when this fails, so no failure can leave a
//! half-materialized package behind.

use super::InstallError;
use super::declaration::{ExpectedAgentDeclaration, verify_agent_declaration};
use crate::limits::package_extract_limits;
use ora_domain::PluginNamespace;
use ora_plugin_manifest::{HookTarget, PluginManifest};
use ora_utils::archive::{ArchiveFormat, extract_archive};
use std::io::Read;
use std::path::Path;

/// Extracts one verified archive, proves the package installable, and commits it atomically.
///
/// Every content check runs against the manifest *inside* the archive rather than the marketplace
/// listing that named it, because that in-package manifest is what discovery reads back after the
/// install. Validating the same source discovery validates is what makes "an installed package is
/// discoverable" an invariant of installation instead of a hope: a missing field, a TOML syntax
/// error, or a kind the package cannot back with files is rejected while the tree is still staged,
/// so nothing is ever committed for it.
///
/// `expected_agent_declaration` carries the listing's `[agent_configuration]` association when
/// it declares one: the package's declaration bytes must then be exactly the ones the release
/// bound, or the package is rejected while it is still staged.
///
/// The staging directory lives inside the destination's parent so the final rename stays on one
/// filesystem, and it is removed when it drops on every failure path.
pub(super) fn materialize_package(
    archive_path: &Path,
    package_dir: &Path,
    package_parent: &Path,
    namespace: &PluginNamespace,
    release_manifest: &PluginManifest,
    selected_target: Option<&HookTarget>,
    expected_agent_declaration: Option<&ExpectedAgentDeclaration>,
) -> Result<(), InstallError> {
    std::fs::create_dir_all(package_parent).map_err(|source| InstallError::Io {
        path: package_parent.to_path_buf(),
        source,
    })?;
    let staging = tempfile::tempdir_in(package_parent).map_err(|source| InstallError::Io {
        path: package_parent.to_path_buf(),
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
    let installed_manifest = read_installed_manifest(staging.path())?;
    ensure_same_identity(release_manifest, &installed_manifest)?;
    crate::validation::validate(
        staging.path(),
        &installed_manifest,
        namespace,
        /*logo*/ None,
    )
    .map_err(InstallError::invalid_package)?;
    // The listing's declaration association, when it declares one, is proven against the
    // package's actual bytes before anything commits: a user's personal configuration is saved
    // against the associated fingerprint, so a package shipping different declaration bytes
    // must never install as that release.
    if let Some(expected) = expected_agent_declaration {
        verify_agent_declaration(staging.path(), expected)?;
    }
    // A targeted archive must self-declare its target in an in-package `[artifact]` section.
    // Missing the declaration fails closed so a wrong-architecture archive cannot install as
    // valid; the already-parsed in-package manifest is reused instead of reading the file twice.
    if let Some(selected) = selected_target {
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
    std::fs::rename(staging.path(), package_dir).map_err(|source| InstallError::Io {
        path: package_dir.to_path_buf(),
        source,
    })
}

/// Reads and parses the `orax.toml` an extracted package ships, as bounded UTF-8 source.
///
/// The byte budget is discovery's manifest budget rather than the extraction budget, which is
/// sized for bundled CLIs: without it a package could make installation read an arbitrarily large
/// file into memory even though discovery would refuse the same manifest afterwards.
pub(crate) fn read_installed_manifest(package_root: &Path) -> Result<PluginManifest, InstallError> {
    let file_name = crate::discovery::MANIFEST_FILE_NAME;
    let manifest_path = package_root.join(file_name);
    let metadata =
        std::fs::symlink_metadata(&manifest_path).map_err(|source| match source.kind() {
            std::io::ErrorKind::NotFound => InstallError::MissingManifest,
            _ => InstallError::Io {
                path: manifest_path.clone(),
                source,
            },
        })?;
    if !metadata.file_type().is_file() {
        return Err(InstallError::invalid_field(
            file_name,
            format!("{file_name} must be a regular file"),
        ));
    }
    let file = std::fs::File::open(&manifest_path).map_err(|source| InstallError::Io {
        path: manifest_path.clone(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(crate::MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| InstallError::Io {
            path: manifest_path.clone(),
            source,
        })?;
    if bytes.len() as u64 > crate::MAX_MANIFEST_BYTES {
        return Err(InstallError::invalid_field(
            file_name,
            format!(
                "{file_name} exceeds the {}-byte limit",
                crate::MAX_MANIFEST_BYTES
            ),
        ));
    }
    let source = String::from_utf8(bytes).map_err(|error| {
        InstallError::invalid_field(
            file_name,
            format!("{file_name} is not valid UTF-8: {error}"),
        )
    })?;
    Ok(PluginManifest::parse_installed(&source)?)
}

/// Rejects a package whose own manifest contradicts the identity its listing published.
///
/// Identity decides the installed directory, the plugin's private data directory, and its Skill
/// rows, so a disagreement is a packaging error rather than a preference to resolve: trusting
/// either side silently would attribute one identity's data to the other. Descriptive metadata
/// (title, description, homepage, license, icon) is deliberately not compared — the package ships
/// the text users read, so the in-package manifest wins there without being a conflict.
fn ensure_same_identity(
    release_manifest: &PluginManifest,
    installed_manifest: &PluginManifest,
) -> Result<(), InstallError> {
    // Compared in this order so the reported field is the one that decides the most: a wrong
    // identifier makes the version and kind comparisons meaningless.
    let identity = [
        (
            "identifier",
            release_manifest.name().to_string(),
            installed_manifest.name().to_string(),
        ),
        (
            "version",
            release_manifest.version().to_string(),
            installed_manifest.version().to_string(),
        ),
        (
            "kind",
            release_manifest.kind().to_string(),
            installed_manifest.kind().to_string(),
        ),
    ];
    for (field, listed, packaged) in identity {
        if listed != packaged {
            return Err(InstallError::invalid_field(
                field,
                format!(
                    "package {field} `{packaged}` does not match the marketplace listing {field} `{listed}`"
                ),
            ));
        }
    }
    Ok(())
}
