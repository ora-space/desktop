use crate::manifest::{AgentManifest, PackageManifest};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use semver::Version;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use thiserror::Error;

const SUPPORTED_MANIFEST_VERSION: u32 = 1;
const SUPPORTED_PLUGIN_API_VERSION: u32 = 1;
const SUPPORTED_AGENT_CONTRACT_VERSION: u32 = 1;

/// Identifies the supported JavaScript module format of an installed package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginPackageType {
    Module,
}

/// Identifies the supported contribution family of an installed plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    Agent,
}

impl PluginKind {
    /// Returns the package manifest spelling used on the frontend wire contract.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
        }
    }
}

/// Holds uninterpreted engine requirements declared by a validated plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEngines {
    pub ora: String,
    pub plugin_api: u32,
    pub bun: String,
}

/// Holds one validated agent contribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPluginAgent {
    pub id: String,
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
    pub kind: PluginKind,
    pub main: PathBuf,
    pub engines: PluginEngines,
    pub agents: Vec<InstalledPluginAgent>,
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

/// Converts a structurally valid package into an installed plugin after semantic checks.
pub(crate) fn validate(
    package_root: &Path,
    manifest: PackageManifest,
) -> Result<InstalledPlugin, ManifestValidationError> {
    require_non_empty("name", &manifest.name)?;
    let version = Version::parse(&manifest.version).map_err(|error| {
        invalid(
            "version",
            format!("package version is not valid SemVer: {error}"),
        )
    })?;
    if manifest.package_type != "module" {
        return Err(invalid("type", "package type must be `module`"));
    }
    if manifest.ora.manifest_version != SUPPORTED_MANIFEST_VERSION {
        return Err(invalid(
            "ora.manifestVersion",
            format!(
                "unsupported manifest version {}; expected {SUPPORTED_MANIFEST_VERSION}",
                manifest.ora.manifest_version
            ),
        ));
    }
    require_non_empty("ora.id", &manifest.ora.id)?;
    require_non_empty("ora.displayName", &manifest.ora.display_name)?;
    let kind = match manifest.ora.kind.as_str() {
        "agent" => PluginKind::Agent,
        value => {
            return Err(invalid(
                "ora.kind",
                format!("unsupported plugin kind `{value}`; expected `agent`"),
            ));
        }
    };
    let main = validate_main_path(package_root, &manifest.ora.main)?;
    require_non_empty("ora.engines.ora", &manifest.ora.engines.ora)?;
    if manifest.ora.engines.plugin_api != SUPPORTED_PLUGIN_API_VERSION {
        return Err(invalid(
            "ora.engines.pluginApi",
            format!(
                "unsupported plugin API version {}; expected {SUPPORTED_PLUGIN_API_VERSION}",
                manifest.ora.engines.plugin_api
            ),
        ));
    }
    require_non_empty("ora.engines.bun", &manifest.ora.engines.bun)?;

    let agents = validate_agents(manifest.ora.contributes.agents)?;

    Ok(InstalledPlugin {
        package_root: package_root.to_path_buf(),
        package_name: manifest.name,
        version,
        package_type: PluginPackageType::Module,
        manifest_version: manifest.ora.manifest_version,
        id: manifest.ora.id,
        display_name: manifest.ora.display_name,
        kind,
        main,
        engines: PluginEngines {
            ora: manifest.ora.engines.ora,
            plugin_api: manifest.ora.engines.plugin_api,
            bun: manifest.ora.engines.bun,
        },
        agents,
    })
}

/// Resolves one existing regular entrypoint without allowing package-boundary escape.
fn validate_main_path(
    package_root: &Path,
    value: &str,
) -> Result<PathBuf, ManifestValidationError> {
    require_non_empty("ora.main", value)?;
    let relative = PortableRelativePath::parse(value).map_err(|error| {
        invalid(
            "ora.main",
            format!("entrypoint must be a safe relative path: {error}"),
        )
    })?;
    if relative.is_root() {
        return Err(invalid(
            "ora.main",
            "entrypoint must identify a package file",
        ));
    }
    let root = CanonicalPathRoot::new(package_root).map_err(|error| {
        invalid(
            "ora.main",
            format!("plugin package root is unavailable: {error}"),
        )
    })?;
    let resolved = root.resolve_existing(&relative).map_err(|error| {
        invalid(
            "ora.main",
            format!("entrypoint must resolve inside the plugin package: {error}"),
        )
    })?;
    // The canonical check covers the current symlink target only; is_file remains path-based and
    // cannot prevent a caller-controlled replacement between validation and later loading.
    if !resolved.is_file() {
        return Err(invalid(
            "ora.main",
            "entrypoint must identify a regular package file",
        ));
    }
    let main = root.relative_path(&resolved).map_err(|error| {
        invalid(
            "ora.main",
            format!("entrypoint must resolve inside the plugin package: {error}"),
        )
    })?;

    Ok(main.to_path_buf())
}

/// Validates agent contract versions and uniqueness inside one package.
fn validate_agents(
    agents: Vec<AgentManifest>,
) -> Result<Vec<InstalledPluginAgent>, ManifestValidationError> {
    let mut seen_ids = HashSet::new();
    let mut installed = Vec::with_capacity(agents.len());

    for agent in agents {
        require_non_empty("ora.contributes.agents[].id", &agent.id)?;
        require_non_empty("ora.contributes.agents[].displayName", &agent.display_name)?;
        if agent.contract_version != SUPPORTED_AGENT_CONTRACT_VERSION {
            return Err(invalid(
                "ora.contributes.agents[].contractVersion",
                format!(
                    "unsupported agent contract version {}; expected {SUPPORTED_AGENT_CONTRACT_VERSION}",
                    agent.contract_version
                ),
            ));
        }
        if !seen_ids.insert(agent.id.clone()) {
            return Err(invalid(
                "ora.contributes.agents[].id",
                format!("duplicate agent id `{}`", agent.id),
            ));
        }

        installed.push(InstalledPluginAgent {
            id: agent.id,
            display_name: agent.display_name,
            contract_version: agent.contract_version,
        });
    }

    Ok(installed)
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
