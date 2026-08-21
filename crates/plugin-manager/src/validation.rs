use crate::ui_validation::{InstalledPluginUi, validate_ui};
use ora_domain::PluginId;
use ora_plugin_manifest::{PluginKind, PluginManifest};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use semver::Version;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Installed orax packages always ship a fixed `main.js` entrypoint at the package root.
pub const INSTALLED_ENTRYPOINT: &str = "main.js";

/// Holds the validated contribution of one installed plugin.
///
/// `kind` selects the variant, and each variant carries everything that kind must declare.
/// Keeping the kind and its contribution in one value is what makes a ui package without
/// surfaces unrepresentable rather than a case every consumer has to re-check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginContribution {
    Agent(InstalledPluginAgent),
    Ui(InstalledPluginUi),
}

impl PluginContribution {
    /// Returns the `kind` spelling used on the frontend wire contract.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Agent(_) => "agent",
            Self::Ui(_) => "ui",
        }
    }
}

/// Holds the single validated agent contributed by one agent-kind package.
///
/// The agent has no identifier of its own: one package provides exactly one agent, so the
/// package's plugin id is that agent's identity everywhere in the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPluginAgent {
    pub display_name: String,
}

/// Holds one fully validated plugin package and its package-local entrypoint.
///
/// `package_root` is the installed package directory (`<data-dir>/plugins/installed/<name>/`);
/// everything the plugin ships is resolved relative to it and nothing below it is ever written
/// by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub package_root: PathBuf,
    pub id: PluginId,
    pub version: Version,
    /// The manifest carries no display name, so the plugin name stands in for every kind; a ui
    /// plugin's user-visible entries are its surface titles rather than the package itself.
    pub display_name: String,
    pub description: String,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub main: PortableRelativePath,
    pub contributes: PluginContribution,
    /// Trusted SVG source for the package icon, absent when the package ships none.
    pub logo: Option<String>,
}

/// Reports a semantic manifest constraint after structural deserialization succeeds.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub(crate) struct ManifestValidationError {
    field_path: String,
    message: String,
}

impl ManifestValidationError {
    /// Returns the stable manifest field associated with the failed constraint.
    pub(crate) fn field_path(&self) -> &str {
        &self.field_path
    }
}

/// Converts a parsed orax manifest into an installed plugin after host-side checks.
///
/// `ora_plugin_manifest` already enforced the schema, so validation here re-checks only what
/// depends on the host or the package on disk: the fixed entrypoint must exist inside the
/// package, the kind must be one the host can run, and ui surfaces must satisfy the navigation
/// and asset policies the surface host enforces later.
///
/// `logo` arrives already read and security-validated by the discovery layer, so this function
/// keeps its filesystem work limited to the entrypoint it must resolve.
pub(crate) fn validate(
    package_root: &Path,
    manifest: &PluginManifest,
    logo: Option<String>,
) -> Result<InstalledPlugin, ManifestValidationError> {
    let name = manifest.name().as_str();
    // Both segments passed the manifest grammar, which is a strict subset of what the domain
    // id accepts, so this conversion cannot fail for a reason the user could act on.
    let id = PluginId::new(manifest.namespace().as_str(), name)
        .map_err(|error| invalid("name", format!("plugin id is not representable: {error}")))?;
    let contributes = match manifest.kind() {
        PluginKind::Agent => PluginContribution::Agent(InstalledPluginAgent {
            display_name: name.to_owned(),
        }),
        PluginKind::Ui => {
            // The manifest crate pairs `kind = "ui"` with a `[ui]` section, so a parsed manifest
            // always takes the `Some` path; the error is reported rather than unwrapped so a
            // future schema change cannot panic discovery.
            let ui = manifest
                .ui()
                .ok_or_else(|| invalid("ui", "a ui plugin must declare a `[ui]` section"))?;
            PluginContribution::Ui(validate_ui(package_root, ui)?)
        }
        PluginKind::Workbench => {
            return Err(invalid(
                "kind",
                "unsupported plugin kind `workbench`; expected `agent` or `ui`",
            ));
        }
    };
    let main = validate_main_path(package_root, INSTALLED_ENTRYPOINT)?;

    Ok(InstalledPlugin {
        package_root: package_root.to_path_buf(),
        id,
        version: manifest.version().clone(),
        display_name: name.to_owned(),
        description: manifest.description().to_owned(),
        homepage: manifest.homepage().map(|url| url.as_str().to_owned()),
        license: manifest.license().map(str::to_owned),
        main,
        contributes,
        logo,
    })
}

/// Resolves one existing regular entrypoint without allowing package-boundary escape.
fn validate_main_path(
    package_root: &Path,
    value: &str,
) -> Result<PortableRelativePath, ManifestValidationError> {
    let relative = PortableRelativePath::parse(value).map_err(|error| {
        invalid(
            "main",
            format!("entrypoint must be a safe relative path: {error}"),
        )
    })?;
    let root = CanonicalPathRoot::new(package_root).map_err(|error| {
        invalid(
            "main",
            format!("plugin package root is unavailable: {error}"),
        )
    })?;
    let resolved = root.resolve_existing(&relative).map_err(|error| {
        invalid(
            "main",
            format!("entrypoint `{value}` must exist inside the plugin package: {error}"),
        )
    })?;
    // The canonical check covers the current symlink target only; is_file remains path-based and
    // cannot prevent a caller-controlled replacement between validation and later loading.
    if !resolved.is_file() {
        return Err(invalid(
            "main",
            format!("entrypoint `{value}` must be a regular package file"),
        ));
    }
    let main = root.relative_path(&resolved).map_err(|error| {
        invalid(
            "main",
            format!("entrypoint must resolve inside the plugin package: {error}"),
        )
    })?;

    Ok(main)
}

/// Builds one semantic error with a stable field path.
pub(crate) fn invalid(
    field_path: impl Into<String>,
    message: impl Into<String>,
) -> ManifestValidationError {
    ManifestValidationError {
        field_path: field_path.into(),
        message: message.into(),
    }
}
