//! Reconciles a marketplace release manifest with the in-package manifest shipped inside an
//! extracted archive, so a wrong or repackaged archive can never be committed as an installed
//! plugin whose on-disk identity disagrees with the registry.
//!
//! The marketplace manifest selects which release to download; the in-package manifest describes
//! the bytes that were actually fetched. These are two independently produced documents, so the
//! four identity fields must agree before the package is renamed into the installed tree. A
//! divergence is a packaging or registry fault rather than a host incompatibility, so it reuses
//! the existing [`InstallError::InvalidPackage`] model instead of introducing a new install state.

use crate::install::InstallError;
use ora_plugin_manifest::PluginManifest;

/// Confirms the in-package manifest names the same plugin the release manifest resolved.
///
/// Each field is checked in declaration order and reported by its manifest field name, so the
/// failure names the exact divergence the registry or packager must correct. `namespace` is
/// checked for completeness even though resolver version 1 admits only `official`: a future
/// resolver admitting more namespaces already has the boundary in place, and namespace is the
/// spec-required fourth identity field rather than speculative behavior.
pub(crate) fn ensure_manifest_identity(
    release: &PluginManifest,
    installed: &PluginManifest,
) -> Result<(), InstallError> {
    if release.namespace() != installed.namespace() {
        return Err(InstallError::InvalidPackage {
            field_path: "namespace".to_owned(),
            message: format!(
                "in-package namespace `{}` does not match release namespace `{}`",
                installed.namespace(),
                release.namespace(),
            ),
        });
    }
    if release.name() != installed.name() {
        return Err(InstallError::InvalidPackage {
            field_path: "identifier".to_owned(),
            message: format!(
                "in-package identifier `{}` does not match release identifier `{}`",
                installed.name(),
                release.name(),
            ),
        });
    }
    if release.version() != installed.version() {
        return Err(InstallError::InvalidPackage {
            field_path: "version".to_owned(),
            message: format!(
                "in-package version `{}` does not match release version `{}`",
                installed.version(),
                release.version(),
            ),
        });
    }
    if release.kind() != installed.kind() {
        return Err(InstallError::InvalidPackage {
            field_path: "kind".to_owned(),
            message: format!(
                "in-package kind `{}` does not match release kind `{}`",
                installed.kind(),
                release.kind(),
            ),
        });
    }
    Ok(())
}
