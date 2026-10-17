//! Verifies a release's declared Agent configuration against the package's actual bytes.
//!
//! A marketplace listing that carries `[agent_configuration]` binds the declaration digest of
//! every package it names. This module is where that binding is proven at install time: the
//! package's in-archive `assets/config.json` is compiled with the same compiler that validated
//! the published static copy, and its fingerprint must equal the associated digest. A mismatch
//! means the user would configure against one schema while another one executes, so the package
//! is rejected while it is still staged.

use super::InstallError;
use ora_plugin_config::{MAX_DECLARATION_BYTES, compile_agent_configuration};
use ora_plugin_manifest::Sha256Digest;
use std::io::Read;
use std::path::Path;

/// Package-relative path of the strict declaration file inside an installed package.
const CONFIGURATION_FILE: &str = "assets/config.json";

/// The declaration identity one release association binds: the declaration schema version and
/// the SHA-256 of the exact declaration bytes every package of that release must carry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedAgentDeclaration {
    schema_version: u32,
    declaration_sha256: Sha256Digest,
}

impl ExpectedAgentDeclaration {
    /// Builds the expectation from a listing's `[agent_configuration]` association.
    pub fn new(schema_version: u32, declaration_sha256: Sha256Digest) -> Self {
        Self {
            schema_version,
            declaration_sha256,
        }
    }

    /// Returns the declaration schema version the association declares.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the declaration digest the association binds.
    pub fn declaration_sha256(&self) -> &Sha256Digest {
        &self.declaration_sha256
    }
}

/// Compiles the package's declaration and proves it is the exact one the release associated.
///
/// The check is deliberately independent of package validation: validation proves the declaration
/// is *legal*, this proves it is *the one the marketplace published*. Both must hold for a
/// package whose listing declares the association.
pub(crate) fn verify_agent_declaration(
    package_root: &Path,
    expected: &ExpectedAgentDeclaration,
) -> Result<(), InstallError> {
    let bytes = read_declaration_bytes(package_root)?;
    let compiled = compile_agent_configuration(&bytes).map_err(|error| {
        InstallError::invalid_field(
            CONFIGURATION_FILE,
            format!("agent configuration does not compile: {error}"),
        )
    })?;
    if compiled.schema_version != expected.schema_version {
        return Err(InstallError::invalid_field(
            CONFIGURATION_FILE,
            format!(
                "package declares agent configuration schema version {}, but the release association declares {}",
                compiled.schema_version, expected.schema_version
            ),
        ));
    }
    if compiled.fingerprint != expected.declaration_sha256.to_string() {
        return Err(InstallError::invalid_field(
            CONFIGURATION_FILE,
            "package declaration digest does not match the release association",
        ));
    }
    Ok(())
}

/// Reads the bounded declaration bytes, refusing anything but a regular file.
///
/// Safe extraction already guarantees containment; the regular-file check here keeps a symlinked
/// declaration (which extraction limits may still permit on some platforms) from being verified
/// through a target that could change after the install commits.
fn read_declaration_bytes(package_root: &Path) -> Result<Vec<u8>, InstallError> {
    let path = package_root.join(CONFIGURATION_FILE);
    let metadata = std::fs::symlink_metadata(&path).map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => InstallError::invalid_field(
            CONFIGURATION_FILE,
            "package carries no agent configuration to verify against the release association",
        ),
        _ => InstallError::Io {
            path: path.clone(),
            source,
        },
    })?;
    if !metadata.file_type().is_file() {
        return Err(InstallError::invalid_field(
            CONFIGURATION_FILE,
            format!("{CONFIGURATION_FILE} must be a regular file"),
        ));
    }
    let file = std::fs::File::open(&path).map_err(|source| InstallError::Io {
        path: path.clone(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_DECLARATION_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| InstallError::Io {
            path: path.clone(),
            source,
        })?;
    if bytes.len() > MAX_DECLARATION_BYTES {
        return Err(InstallError::invalid_field(
            CONFIGURATION_FILE,
            format!("{CONFIGURATION_FILE} exceeds the {MAX_DECLARATION_BYTES}-byte limit"),
        ));
    }
    Ok(bytes)
}
