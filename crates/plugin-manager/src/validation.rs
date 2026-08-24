use crate::webview::{InstalledWebviewDescriptor, validate_webview};
use crate::workbench::{InstalledWorkbenchDescriptor, validate_workbench};
use ora_domain::PluginId;
use ora_plugin_manifest::{PluginKind, PluginManifest};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use semver::Version;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Installed orax packages with a process always ship a fixed `main.js` at the package root.
pub const INSTALLED_ENTRYPOINT: &str = "main.js";

/// Holds the validated contribution of one installed plugin.
///
/// `kind` selects the variant, and each variant carries everything that kind must declare,
/// including the process entrypoint for the kinds that have one. Keeping the kind and its
/// contribution in one value is what makes "a webview plugin with an entrypoint" or "a
/// workbench plugin without a page" unrepresentable rather than a case every consumer re-checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginContribution {
    Agent(InstalledPluginAgent),
    Workbench(InstalledWorkbenchDescriptor),
    Webview(InstalledWebviewDescriptor),
}

impl PluginContribution {
    /// Returns the `kind` spelling used on the frontend wire contract.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Agent(_) => "agent",
            Self::Workbench(_) => "workbench",
            Self::Webview(_) => "webview",
        }
    }

    /// Returns the process entrypoint relative to the package root, if this kind runs one.
    pub fn entrypoint(&self) -> Option<&PortableRelativePath> {
        match self {
            Self::Agent(agent) => Some(&agent.entrypoint),
            Self::Workbench(workbench) => Some(&workbench.entrypoint),
            Self::Webview(_) => None,
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
    /// `main.js` relative to the package root.
    pub entrypoint: PortableRelativePath,
}

/// Holds one fully validated plugin package.
///
/// `package_root` is the installed package directory
/// (`<data-dir>/plugins/installed/<namespace>/<name>/<version>/`); everything the plugin ships
/// is resolved relative to it and nothing below it is ever written by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub package_root: PathBuf,
    pub id: PluginId,
    pub version: Version,
    /// The manifest carries no display name, so the plugin name stands in for every kind.
    pub display_name: String,
    pub description: String,
    pub homepage: Option<String>,
    pub license: Option<String>,
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
/// depends on the host or the package on disk: the entrypoint and page files a kind must ship,
/// the files a kind must not ship, and the cross-value policies of the kind-specific sections.
///
/// `logo` arrives already read and security-validated by the discovery layer, so this function
/// keeps its filesystem work limited to the files it must resolve.
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
            entrypoint: validate_entrypoint(package_root)?,
        }),
        PluginKind::Workbench => {
            PluginContribution::Workbench(validate_workbench(package_root, manifest.workbench())?)
        }
        PluginKind::Webview => {
            // The manifest crate pairs `kind = "webview"` with a `[webview]` section, so a parsed
            // manifest always takes the `Some` path; the error is reported rather than unwrapped
            // so a future schema change cannot panic discovery.
            let webview = manifest.webview().ok_or_else(|| {
                invalid(
                    "webview",
                    "a webview plugin must declare a `[webview]` section",
                )
            })?;
            PluginContribution::Webview(validate_webview(package_root, webview)?)
        }
    };

    Ok(InstalledPlugin {
        package_root: package_root.to_path_buf(),
        id,
        version: manifest.version().clone(),
        display_name: name.to_owned(),
        description: manifest.description().to_owned(),
        homepage: manifest.homepage().map(|url| url.as_str().to_owned()),
        license: manifest.license().map(str::to_owned),
        contributes,
        logo,
    })
}

/// Resolves the fixed `main.js` entrypoint as an existing regular file inside the package.
pub(crate) fn validate_entrypoint(
    package_root: &Path,
) -> Result<PortableRelativePath, ManifestValidationError> {
    validate_main_path(package_root, INSTALLED_ENTRYPOINT)
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
