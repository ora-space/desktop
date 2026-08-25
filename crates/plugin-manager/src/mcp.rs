//! Host-side validation of the `mcp` kind: a configuration-only package whose single artifact
//! is the compiled MCP Configuration from `assets/config.json`.

use crate::validation::{INSTALLED_ENTRYPOINT, ManifestValidationError, invalid};
use ora_plugin_config::{
    CompileConfigurationFileError, CompiledConfigurationFile, CompiledMcpConfiguration,
    McpStdioTransport, McpTransport,
};
use ora_utils::path::CanonicalPathRoot;
use std::path::Path;

/// Package-relative path of the MCP configuration file mandated by the spec.
pub const MCP_CONFIGURATION_FILE: &str = "assets/config.json";

/// Holds the Installed MCP Descriptor of one mcp-kind package.
///
/// The descriptor proves the package is statically valid. It is not a `ResolvedMcp`: it says
/// nothing about the user having filled Settings or any Agent having loaded the MCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledMcpDescriptor {
    pub configuration: CompiledMcpConfiguration,
}

/// Validates one mcp-kind package around its already-compiled configuration file.
///
/// An MCP package must not look runnable (no `main.js`), must ship the mandated configuration
/// file in the MCP shape, and — for a stdio transport — must contain the declared command as a
/// real executable file inside the package.
pub(crate) fn validate_mcp(
    package_root: &Path,
    configuration_file: &Result<Option<CompiledConfigurationFile>, CompileConfigurationFileError>,
) -> Result<InstalledMcpDescriptor, ManifestValidationError> {
    // Same policy as webview: an entrypoint would suggest a process the host never starts and
    // would be a silent way to smuggle code into a config-only kind.
    if package_root.join(INSTALLED_ENTRYPOINT).exists() {
        return Err(invalid(
            "kind",
            format!("an mcp plugin must not ship `{INSTALLED_ENTRYPOINT}`"),
        ));
    }
    let configuration = match configuration_file {
        // Unlike the Settings-only kinds, a broken declaration rejects the whole package: the
        // configuration file is the entire contribution of an MCP package.
        Err(error) => {
            return Err(invalid(
                MCP_CONFIGURATION_FILE,
                format!("MCP configuration is invalid: {error}"),
            ));
        }
        Ok(None) => {
            return Err(invalid(
                MCP_CONFIGURATION_FILE,
                format!("an mcp plugin must ship `{MCP_CONFIGURATION_FILE}`"),
            ));
        }
        Ok(Some(CompiledConfigurationFile::Settings(_))) => {
            return Err(invalid(
                MCP_CONFIGURATION_FILE,
                "an mcp plugin must declare exactly one `transport`",
            ));
        }
        Ok(Some(CompiledConfigurationFile::Mcp(configuration))) => configuration.clone(),
    };
    if let McpTransport::Stdio(stdio) = &configuration.transport {
        validate_command_containment(package_root, stdio)?;
    }

    Ok(InstalledMcpDescriptor { configuration })
}

/// Confirms the compiled stdio command resolves to a regular executable file that stays inside
/// this exact installed package version, refusing symlink or reparse-point escapes.
fn validate_command_containment(
    package_root: &Path,
    stdio: &McpStdioTransport,
) -> Result<(), ManifestValidationError> {
    let command = stdio.command.as_str();
    let root = CanonicalPathRoot::new(package_root).map_err(|error| {
        invalid(
            "transport.command",
            format!("plugin package root is unavailable: {error}"),
        )
    })?;
    let resolved = root.resolve_existing(&stdio.command).map_err(|error| {
        invalid(
            "transport.command",
            format!("command `{command}` must exist inside the plugin package: {error}"),
        )
    })?;
    // The canonical check covers the current symlink target only; is_file remains path-based and
    // cannot prevent a caller-controlled replacement between validation and later use.
    if !resolved.is_file() {
        return Err(invalid(
            "transport.command",
            format!("command `{command}` must be a regular package file"),
        ));
    }
    root.relative_path(&resolved).map_err(|error| {
        invalid(
            "transport.command",
            format!("command must resolve inside the plugin package: {error}"),
        )
    })?;
    // Windows decides executability by extension and ACLs at spawn time, so only Unix has a
    // static mode bit worth checking at install time.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = resolved
            .metadata()
            .map_err(|error| {
                invalid(
                    "transport.command",
                    format!("command `{command}` metadata is unavailable: {error}"),
                )
            })?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(invalid(
                "transport.command",
                format!("command `{command}` must be executable"),
            ));
        }
    }

    Ok(())
}
