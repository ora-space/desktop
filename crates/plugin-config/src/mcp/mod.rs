//! Compiles one MCP Plugin's `assets/config.json` — the optional Settings subset plus the
//! exclusive MCP Transport — into an immutable MCP Configuration.
//!
//! The compiled value is static install-time truth only: it proves the declaration is legal, not
//! that the user filled Settings, that a remote endpoint is reachable, or that any Agent loaded
//! the MCP. Resolution against `store.json` (`ResolvedMcp`) is a later, separate step and is
//! deliberately not modeled here.

#[cfg(test)]
mod tests;
mod transport;

use crate::declaration::{
    CompileDeclarationError, CompiledDeclaration, MAX_DECLARATION_BYTES, compile_declaration,
    compile_declaration_from_value, parse_strict_json,
};
use ora_utils::path::PortableRelativePath;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;
use transport::compile_transport;
use url::Url;

/// The package-relative directory an MCP stdio command must live in.
pub const MCP_COMMAND_DIRECTORY: &str = "assets/";

/// Distinguishes the two strict `assets/config.json` shapes by the `transport` member.
///
/// A Settings-only declaration rejects a `transport` member (`deny_unknown_fields`) and an MCP
/// Configuration requires one, so the presence of that member decides the schema without any
/// caller-provided kind hint. Kind policy — an MCP package must ship the MCP shape and every
/// other kind must not — stays with the package validator that knows the manifest kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompiledConfigurationFile {
    Settings(CompiledDeclaration),
    Mcp(CompiledMcpConfiguration),
}

/// Holds one validated MCP Configuration compiled from `assets/config.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledMcpConfiguration {
    pub schema_version: u32,
    /// The user-facing Settings subset, absent when the package declares no Settings.
    ///
    /// This is the exact declaration the existing Plugin Configuration editor consumes, so an
    /// MCP package feeds the settings UI without a second declaration format.
    pub settings: Option<CompiledDeclaration>,
    pub transport: McpTransport,
}

/// Models the exclusive MCP Transport so illegal combinations are unrepresentable: stdio cannot
/// carry a URL or headers, HTTP cannot carry a command, args, or env.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransport {
    Stdio(McpStdioTransport),
    Http(McpHttpTransport),
}

/// Describes one package-contained stdio MCP Server launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpStdioTransport {
    /// Package-relative executable under `assets/`; filesystem containment is re-checked by the
    /// package validator that owns the package root.
    pub command: PortableRelativePath,
    pub args: Vec<McpArgument>,
    pub env: BTreeMap<String, McpValueExpression>,
}

/// Describes one remote MCP Streamable HTTP endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHttpTransport {
    pub url: Url,
    pub headers: BTreeMap<String, McpValueExpression>,
}

/// One stdio argument: a resolvable value or the authoritative workspace directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpArgument {
    Value(McpValueExpression),
    /// `{ "context": "workspace" }`, resolved later to the Agent instance's authoritative cwd.
    WorkspaceContext,
}

/// One value that resolves to a string when the MCP is used.
///
/// Number and boolean literals are canonicalized to strings at compile time because every
/// target position (argument, environment value, header value) is a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpValueExpression {
    Literal(String),
    Setting {
        id: String,
        prefix: String,
        suffix: String,
    },
}

/// Reports an MCP Configuration that cannot be compiled without ambiguity.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompileMcpConfigurationError {
    #[error(transparent)]
    Declaration(#[from] CompileDeclarationError),
    #[error("MCP configuration does not match schema version one: {0}")]
    InvalidStructure(String),
    #[error("unsupported MCP configuration schema version {0}")]
    UnsupportedSchemaVersion(u32),
    // Phase one stores API keys as `string` Settings, so the reserved spec types fail with
    // a targeted message instead of a generic unknown-variant error.
    #[error(
        "invalid Setting `{setting_id}`: type `{found}` is not supported by MCP configuration schema version one"
    )]
    UnsupportedSettingType { setting_id: String, found: String },
    #[error("unsupported MCP transport type `{0}`")]
    UnsupportedTransportType(String),
    #[error("invalid MCP transport `{field}`: {reason}")]
    InvalidTransport { field: String, reason: String },
}

/// Reports either strict `assets/config.json` shape failing to compile.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompileConfigurationFileError {
    #[error(transparent)]
    Settings(#[from] CompileDeclarationError),
    #[error(transparent)]
    Mcp(#[from] CompileMcpConfigurationError),
}

/// Compiles one `assets/config.json` payload into whichever strict shape it declares.
pub fn compile_configuration_file(
    source: &[u8],
) -> Result<CompiledConfigurationFile, CompileConfigurationFileError> {
    if source.len() > MAX_DECLARATION_BYTES {
        return Err(CompileDeclarationError::TooLarge.into());
    }
    let value = parse_strict_json(source).map_err(CompileConfigurationFileError::Settings)?;
    if value
        .as_object()
        .is_some_and(|object| object.contains_key("transport"))
    {
        compile_mcp_configuration(value)
            .map(CompiledConfigurationFile::Mcp)
            .map_err(CompileConfigurationFileError::Mcp)
    } else {
        compile_declaration(source)
            .map(CompiledConfigurationFile::Settings)
            .map_err(CompileConfigurationFileError::Settings)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawMcpConfiguration {
    schema_version: u32,
    #[serde(default)]
    settings: Option<Value>,
    transport: Value,
}

/// Compiles one duplicate-free MCP configuration JSON value.
fn compile_mcp_configuration(
    value: Value,
) -> Result<CompiledMcpConfiguration, CompileMcpConfigurationError> {
    let raw: RawMcpConfiguration = serde_json::from_value(value)
        .map_err(|error| CompileMcpConfigurationError::InvalidStructure(error.to_string()))?;
    if raw.schema_version != 1 {
        return Err(CompileMcpConfigurationError::UnsupportedSchemaVersion(
            raw.schema_version,
        ));
    }
    let settings = raw.settings.map(compile_settings_subset).transpose()?;
    let declared_ids: Vec<String> = settings
        .iter()
        .flat_map(|declaration| &declaration.settings)
        .map(|setting| setting.id.clone())
        .collect();
    let transport = compile_transport(raw.transport, &declared_ids)?;

    Ok(CompiledMcpConfiguration {
        schema_version: 1,
        settings,
        transport,
    })
}

/// Compiles the Settings member by delegating to the shared Settings-only declaration compiler.
fn compile_settings_subset(
    settings: Value,
) -> Result<CompiledDeclaration, CompileMcpConfigurationError> {
    // Reserved spec types are rejected up front so the author reads the phase-one policy
    // instead of serde's unknown-variant wording.
    if let Value::Object(entries) = &settings {
        for (setting_id, declaration) in entries {
            if let Some(found) = declaration.get("type").and_then(Value::as_str)
                && matches!(found, "secret" | "file" | "directory")
            {
                return Err(CompileMcpConfigurationError::UnsupportedSettingType {
                    setting_id: setting_id.clone(),
                    found: found.to_owned(),
                });
            }
        }
    }
    let wrapped = serde_json::json!({
        "schemaVersion": 1,
        "settings": settings,
    });
    Ok(compile_declaration_from_value(wrapped)?)
}
