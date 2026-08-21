use ora_plugin_manifest::{PluginKind, PluginManifest};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use semver::Version;
use std::path::{Path, PathBuf};
use thiserror::Error;

const SUPPORTED_PLUGIN_API_VERSION: u32 = 1;
const SUPPORTED_AGENT_CONTRACT_VERSION: u32 = 1;
/// Installed orax packages always ship a fixed `main.js` entrypoint.
const INSTALLED_ENTRYPOINT: &str = "main.js";

/// Identifies the supported JavaScript module format of an installed package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginPackageType {
    Module,
}

/// Holds the validated contribution of one installed plugin.
///
/// `ora.kind` selects the variant, and each variant carries everything that kind must declare.
/// Keeping the kind and its contribution in one value is what makes an agent package without an
/// agent declaration unrepresentable rather than a case every consumer has to re-check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginContribution {
    Agent(InstalledPluginAgent),
}

impl PluginContribution {
    /// Returns the `ora.kind` spelling used on the frontend wire contract.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Agent(_) => "agent",
        }
    }
}

/// Holds uninterpreted engine requirements declared by a validated plugin.
///
/// Orax manifests do not declare engine requirements, so the host defaults are kept here to
/// satisfy the shared contract; no consumer currently interprets these values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEngines {
    pub ora: String,
    pub plugin_api: u32,
    pub bun: String,
}

/// Holds the single validated agent contributed by one agent-kind package.
///
/// The agent has no identifier of its own: one package provides exactly one agent, so the
/// package's identity is that agent's identity everywhere in the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPluginAgent {
    pub display_name: String,
    pub contract_version: u32,
}

/// Holds one fully validated plugin package and its package-local entrypoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub package_root: PathBuf,
    pub package_name: String,
    pub version: Version,
    pub package_type: PluginPackageType,
    pub manifest_version: u32,
    pub id: String,
    pub display_name: String,
    pub main: PortableRelativePath,
    pub engines: PluginEngines,
    pub contributes: PluginContribution,
    /// Trusted SVG source for the package icon, absent when the package ships none.
    pub logo: Option<String>,
}

/// Reports a semantic manifest constraint after structural deserialization succeeds.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub(crate) struct ManifestValidationError {
    field_path: &'static str,
    message: String,
}

impl ManifestValidationError {
    /// Returns the stable manifest field associated with the failed constraint.
    pub(crate) fn field_path(&self) -> &'static str {
        self.field_path
    }
}

/// Converts a structurally valid orax package into an installed plugin after semantic checks.
///
/// The discovery layer already parsed the manifest with `ora_plugin_manifest`, so validation here
/// only re-checks the runtime invariants: the entrypoint must exist inside the package and the
/// kind must be one the host can run. Fields the orax schema omits (`display_name`, engines) fall
/// back to stable host defaults because the rest of the codebase does not interpret them.
///
/// `logo` arrives already read and security-validated by the discovery layer, so this function
/// keeps its filesystem work limited to the entrypoint it must resolve.
pub(crate) fn validate(
    package_root: &Path,
    manifest: &PluginManifest,
    logo: Option<String>,
) -> Result<InstalledPlugin, ManifestValidationError> {
    let name = manifest.name().as_str().to_owned();
    let id = format!(
        "{}/{}",
        manifest.namespace().as_str(),
        manifest.name().as_str()
    );
    let contributes = validate_contribution(&name, manifest.kind())?;
    let main = validate_main_path(package_root, INSTALLED_ENTRYPOINT)?;

    Ok(InstalledPlugin {
        package_root: package_root.to_path_buf(),
        package_name: name.clone(),
        version: manifest.version().clone(),
        package_type: PluginPackageType::Module,
        manifest_version: manifest.resolver() as u32,
        id,
        display_name: name,
        main,
        engines: PluginEngines {
            ora: String::new(),
            plugin_api: SUPPORTED_PLUGIN_API_VERSION,
            bun: String::new(),
        },
        contributes,
        logo,
    })
}

/// Resolves one existing regular entrypoint without allowing package-boundary escape.
fn validate_main_path(
    package_root: &Path,
    value: &str,
) -> Result<PortableRelativePath, ManifestValidationError> {
    require_non_empty("main", value)?;
    let relative = PortableRelativePath::parse(value).map_err(|error| {
        invalid(
            "main",
            format!("entrypoint must be a safe relative path: {error}"),
        )
    })?;
    if relative.is_root() {
        return Err(invalid("main", "entrypoint must identify a package file"));
    }
    let root = CanonicalPathRoot::new(package_root).map_err(|error| {
        invalid(
            "main",
            format!("plugin package root is unavailable: {error}"),
        )
    })?;
    let resolved = root.resolve_existing(&relative).map_err(|error| {
        invalid(
            "main",
            format!("entrypoint must resolve inside the plugin package: {error}"),
        )
    })?;
    if !resolved.is_file() {
        return Err(invalid(
            "main",
            "entrypoint must identify a regular package file",
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

/// Pairs the declared kind with the contribution that kind is required to carry.
fn validate_contribution(
    display_name: &str,
    kind: PluginKind,
) -> Result<PluginContribution, ManifestValidationError> {
    match kind {
        PluginKind::Agent => Ok(PluginContribution::Agent(InstalledPluginAgent {
            display_name: display_name.to_owned(),
            contract_version: SUPPORTED_AGENT_CONTRACT_VERSION,
        })),
        PluginKind::Workbench => Err(invalid(
            "kind",
            "unsupported plugin kind `workbench`; expected `agent`",
        )),
    }
}

/// Rejects required strings that contain only whitespace while preserving valid values verbatim.
fn require_non_empty(field_path: &'static str, value: &str) -> Result<(), ManifestValidationError> {
    if value.trim().is_empty() {
        return Err(invalid(field_path, "value must not be empty"));
    }

    Ok(())
}

/// Builds one semantic error with a stable field path.
fn invalid(field_path: &'static str, message: impl Into<String>) -> ManifestValidationError {
    ManifestValidationError {
        field_path,
        message: message.into(),
    }
}
