use crate::hook::{InstalledHookDescriptor, validate_hook};
use crate::mcp::{InstalledMcpDescriptor, validate_mcp};
use crate::skill::{InstalledSkillDescriptor, validate_skill};
use crate::webview::{InstalledWebviewDescriptor, validate_webview};
use crate::workbench::{InstalledWorkbenchDescriptor, validate_workbench};
use ora_domain::{PluginId, PluginNamespace};
use ora_plugin_config::{CompiledConfigurationFile, ConfigurationError, ConfigurationService};
use ora_plugin_manifest::{PluginKind, PluginManifest};
use ora_utils::path::{CanonicalPathRoot, PortableRelativePath};
use semver::Version;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Package-relative path of the strict `assets/config.json` contribution file shared by the
/// processless MCP and Hook kinds.
pub const CONFIGURATION_FILE: &str = "assets/config.json";

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
    Skill(InstalledSkillDescriptor),
    Mcp(InstalledMcpDescriptor),
    Hook(InstalledHookDescriptor),
}

impl PluginContribution {
    /// Returns the `kind` spelling used on the frontend wire contract.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Agent(_) => "agent",
            Self::Workbench(_) => "workbench",
            Self::Webview(_) => "webview",
            Self::Skill(_) => "skill",
            Self::Mcp(_) => "mcp",
            Self::Hook(_) => "hook",
        }
    }

    /// Returns the process entrypoint relative to the package root, if this kind runs one.
    pub fn entrypoint(&self) -> Option<&PortableRelativePath> {
        match self {
            Self::Agent(agent) => Some(&agent.entrypoint),
            Self::Workbench(workbench) => Some(&workbench.entrypoint),
            Self::Webview(_) | Self::Skill(_) | Self::Mcp(_) | Self::Hook(_) => None,
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
    pub configuration_declaration: PluginConfigurationDeclarationValidity,
}

/// Records whether an immutable configuration declaration can participate in installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginConfigurationDeclarationValidity {
    NotDeclared,
    Valid,
    Invalid { reason: String },
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
/// `namespace` is supplied by the host — the installing source, or the directory an already
/// installed package sits under — and is never read from the manifest: a package is third-party
/// content, and a namespace it could name for itself would let it claim another source's
/// directories, configuration, and Skill rows.
///
/// `logo` arrives already read and security-validated by the discovery layer, so this function
/// keeps its filesystem work limited to the files it must resolve.
pub(crate) fn validate(
    package_root: &Path,
    manifest: &PluginManifest,
    namespace: &PluginNamespace,
    logo: Option<String>,
) -> Result<InstalledPlugin, ManifestValidationError> {
    let name = manifest.name().as_str();
    // The name passed the manifest grammar, which is a strict subset of what the domain id
    // accepts, and the namespace is already a validated segment, so this conversion cannot fail
    // for a reason the user could act on.
    let id = PluginId::new(namespace.clone(), name).map_err(|error| {
        invalid(
            "identifier",
            format!("plugin id is not representable: {error}"),
        )
    })?;
    // The one bounded read of `assets/config.json` serves both the kind policy below and the
    // Settings-declaration validity, so the two views can never disagree about the same file.
    let configuration_file =
        match ConfigurationService::configuration_file_from_package(package_root) {
            Ok(file) => Ok(file),
            Err(ConfigurationError::InvalidDeclaration(error)) => Err(error),
            Err(error) => {
                return Err(invalid(
                    CONFIGURATION_FILE,
                    format!("configuration declaration could not be read: {error}"),
                ));
            }
        };
    // The MCP shape is exclusive to the mcp kind: any other kind shipping a transport is either
    // a mispackaged MCP or an attempt to smuggle connection state into a process kind. The Hook
    // shape is exclusive to the hook kind for the symmetric reason.
    if !matches!(manifest.kind(), PluginKind::Mcp)
        && matches!(
            configuration_file,
            Ok(Some(CompiledConfigurationFile::Mcp(_)))
        )
    {
        return Err(invalid(
            CONFIGURATION_FILE,
            format!(
                "only `mcp` packages may declare an MCP transport (kind is `{}`)",
                manifest.kind()
            ),
        ));
    }
    if !matches!(manifest.kind(), PluginKind::Hook)
        && matches!(
            configuration_file,
            Ok(Some(CompiledConfigurationFile::Hook(_)))
        )
    {
        return Err(invalid(
            CONFIGURATION_FILE,
            format!(
                "only `hook` packages may declare a Hook configuration (kind is `{}`)",
                manifest.kind()
            ),
        ));
    }
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
        PluginKind::Skill => PluginContribution::Skill(validate_skill(package_root)?),
        PluginKind::Mcp => {
            PluginContribution::Mcp(validate_mcp(package_root, &configuration_file)?)
        }
        PluginKind::Hook => PluginContribution::Hook(validate_hook(
            package_root,
            &configuration_file,
            manifest.artifact(),
        )?),
    };
    let configuration_declaration = match &configuration_file {
        Ok(None) => PluginConfigurationDeclarationValidity::NotDeclared,
        Ok(Some(CompiledConfigurationFile::Settings(_))) => {
            PluginConfigurationDeclarationValidity::Valid
        }
        // An MCP file contributes exactly its Settings subset to the configuration editor, so a
        // transport-only file behaves like a package without a declaration.
        Ok(Some(CompiledConfigurationFile::Mcp(configuration))) => {
            if configuration.settings.is_some() {
                PluginConfigurationDeclarationValidity::Valid
            } else {
                PluginConfigurationDeclarationValidity::NotDeclared
            }
        }
        // A Hook file contributes its optional Settings subset to the configuration editor; RTK
        // v0.1.0 declares none, so no Configure action is offered until Settings ship.
        Ok(Some(CompiledConfigurationFile::Hook(configuration))) => {
            if configuration.settings.is_some() {
                PluginConfigurationDeclarationValidity::Valid
            } else {
                PluginConfigurationDeclarationValidity::NotDeclared
            }
        }
        Err(error) => PluginConfigurationDeclarationValidity::Invalid {
            reason: error.to_string(),
        },
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
        configuration_declaration,
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
