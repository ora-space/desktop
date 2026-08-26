//! Compiles one MCP Plugin's `assets/config.json` — the optional Settings subset plus the
//! exclusive MCP Transport — into an immutable MCP Configuration.
//!
//! The compiled value is static install-time truth only: it proves the declaration is legal, not
//! that the user filled Settings, that a remote endpoint is reachable, or that any Agent loaded
//! the MCP. Resolution against `store.json` (`ResolvedMcp`) is a later, separate step and is
//! deliberately not modeled here.

use crate::declaration::{
    CompileDeclarationError, CompiledDeclaration, MAX_DECLARATION_BYTES, compile_declaration,
    compile_declaration_from_value, parse_strict_json,
};
use ora_utils::path::PortableRelativePath;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use thiserror::Error;
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
    // Phase one deliberately stores API keys as `string` Settings (see
    // docs/adr/0001-phase-1-mcp-api-keys-are-strings.md), so the reserved spec types fail with
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawStdioTransport {
    #[serde(rename = "type")]
    _transport_type: String,
    command: String,
    #[serde(default)]
    args: Vec<Value>,
    #[serde(default)]
    env: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHttpTransport {
    #[serde(rename = "type")]
    _transport_type: String,
    url: String,
    #[serde(default)]
    headers: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettingReference {
    setting: String,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    suffix: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContextReference {
    context: String,
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

/// Dispatches the exclusive transport member on its required `type` discriminator.
fn compile_transport(
    transport: Value,
    declared_ids: &[String],
) -> Result<McpTransport, CompileMcpConfigurationError> {
    let Some(transport_type) = transport.get("type").and_then(Value::as_str) else {
        return Err(invalid_transport(
            "transport.type",
            "transport must declare a `type` string",
        ));
    };
    match transport_type {
        "stdio" => compile_stdio_transport(transport, declared_ids),
        "http" => compile_http_transport(transport, declared_ids),
        found => Err(CompileMcpConfigurationError::UnsupportedTransportType(
            found.to_owned(),
        )),
    }
}

/// Compiles the stdio transport shape: package-contained command, args, and env bindings.
fn compile_stdio_transport(
    transport: Value,
    declared_ids: &[String],
) -> Result<McpTransport, CompileMcpConfigurationError> {
    let raw: RawStdioTransport = serde_json::from_value(transport)
        .map_err(|error| invalid_transport("transport", error.to_string()))?;
    let command = compile_command(&raw.command)?;
    let args = raw
        .args
        .into_iter()
        .enumerate()
        .map(|(index, argument)| compile_argument(index, argument, declared_ids))
        .collect::<Result<Vec<_>, _>>()?;
    let env = raw
        .env
        .into_iter()
        .map(|(name, binding)| {
            let field = format!("transport.env.{name}");
            validate_environment_name(&name, &field)?;
            let expression = compile_value_expression(binding, &field, declared_ids)?;
            Ok((name, expression))
        })
        .collect::<Result<BTreeMap<_, _>, CompileMcpConfigurationError>>()?;

    Ok(McpTransport::Stdio(McpStdioTransport {
        command,
        args,
        env,
    }))
}

/// Compiles the HTTP transport shape: an HTTPS Streamable HTTP endpoint plus header bindings.
fn compile_http_transport(
    transport: Value,
    declared_ids: &[String],
) -> Result<McpTransport, CompileMcpConfigurationError> {
    let raw: RawHttpTransport = serde_json::from_value(transport)
        .map_err(|error| invalid_transport("transport", error.to_string()))?;
    let url = Url::parse(&raw.url)
        .map_err(|error| invalid_transport("transport.url", format!("invalid URL: {error}")))?;
    // Development-mode localhost HTTP is not plumbed in this slice, so the rule is simply HTTPS.
    if url.scheme() != "https" {
        return Err(invalid_transport(
            "transport.url",
            "URL scheme must be HTTPS",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid_transport(
            "transport.url",
            "URL must not contain a username or password",
        ));
    }
    if url.fragment().is_some() {
        return Err(invalid_transport(
            "transport.url",
            "URL must not contain a fragment",
        ));
    }
    // Phase 1 forbids every query parameter so credentials cannot be smuggled outside header
    // Setting references; Tavily documents a query-key option but Ora still refuses it.
    if url.query().is_some() {
        return Err(invalid_transport(
            "transport.url",
            "URL must not contain a query string",
        ));
    }
    let headers = raw
        .headers
        .into_iter()
        .map(|(name, binding)| {
            let field = format!("transport.headers.{name}");
            validate_header_name(&name, &field)?;
            let expression = compile_header_expression(binding, &field, declared_ids)?;
            Ok((name, expression))
        })
        .collect::<Result<BTreeMap<_, _>, CompileMcpConfigurationError>>()?;

    Ok(McpTransport::Http(McpHttpTransport { url, headers }))
}

/// Validates the stdio command as a normalized package path under `assets/`.
///
/// PATH lookup (`npx`, `uvx`, shells) is unrepresentable by construction: the value must be a
/// traversal-free relative path with at least one component below `assets/`.
fn compile_command(command: &str) -> Result<PortableRelativePath, CompileMcpConfigurationError> {
    let parsed = PortableRelativePath::parse(command).map_err(|error| {
        invalid_transport(
            "transport.command",
            format!("command must be a safe package-relative path: {error}"),
        )
    })?;
    let is_contained = parsed
        .as_str()
        .strip_prefix(MCP_COMMAND_DIRECTORY)
        .is_some_and(|remainder| !remainder.is_empty());
    if !is_contained {
        return Err(invalid_transport(
            "transport.command",
            format!("command must name a file below `{MCP_COMMAND_DIRECTORY}`"),
        ));
    }
    Ok(parsed)
}

/// Compiles one stdio argument: a literal, a Setting reference, or the workspace context.
fn compile_argument(
    index: usize,
    argument: Value,
    declared_ids: &[String],
) -> Result<McpArgument, CompileMcpConfigurationError> {
    let field = format!("transport.args[{index}]");
    if argument
        .as_object()
        .is_some_and(|object| object.contains_key("context"))
    {
        let reference: RawContextReference = serde_json::from_value(argument)
            .map_err(|error| invalid_transport(&field, error.to_string()))?;
        if reference.context != "workspace" {
            return Err(invalid_transport(
                &field,
                format!("unknown context `{}`", reference.context),
            ));
        }
        return Ok(McpArgument::WorkspaceContext);
    }
    compile_value_expression(argument, &field, declared_ids).map(McpArgument::Value)
}

/// Compiles one literal or Setting-reference value used by args and env.
fn compile_value_expression(
    value: Value,
    field: &str,
    declared_ids: &[String],
) -> Result<McpValueExpression, CompileMcpConfigurationError> {
    match value {
        Value::String(literal) => Ok(McpValueExpression::Literal(literal)),
        // Non-string scalars canonicalize at compile time because the target is always a string.
        Value::Number(literal) => Ok(McpValueExpression::Literal(literal.to_string())),
        Value::Bool(literal) => Ok(McpValueExpression::Literal(literal.to_string())),
        Value::Object(object) => compile_setting_reference(object, field, declared_ids),
        Value::Null | Value::Array(_) => Err(invalid_transport(
            field,
            "value must be a scalar literal or a `{ \"setting\": ... }` reference",
        )),
    }
}

/// Compiles one HTTP header value.
///
/// Phase 1 only accepts Setting references: a string literal would be a way to bake an API key
/// into the immutable package, which the Tavily loop and ADR-0001 both forbid.
fn compile_header_expression(
    value: Value,
    field: &str,
    declared_ids: &[String],
) -> Result<McpValueExpression, CompileMcpConfigurationError> {
    match value {
        Value::Object(object) => compile_setting_reference(object, field, declared_ids),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::String(_) => {
            Err(invalid_transport(
                field,
                "header value must be a `{ \"setting\": ... }` reference",
            ))
        }
    }
}

/// Compiles one `{ "setting": <id>, "prefix"?, "suffix"? }` reference against declared Settings.
fn compile_setting_reference(
    object: Map<String, Value>,
    field: &str,
    declared_ids: &[String],
) -> Result<McpValueExpression, CompileMcpConfigurationError> {
    let reference: RawSettingReference = serde_json::from_value(Value::Object(object))
        .map_err(|error| invalid_transport(field, error.to_string()))?;
    if !declared_ids.contains(&reference.setting) {
        return Err(invalid_transport(
            field,
            format!("references undeclared Setting `{}`", reference.setting),
        ));
    }
    for (name, text) in [("prefix", &reference.prefix), ("suffix", &reference.suffix)] {
        validate_bound_text(text, &format!("{field}.{name}"))?;
    }
    Ok(McpValueExpression::Setting {
        id: reference.setting,
        prefix: reference.prefix,
        suffix: reference.suffix,
    })
}

/// Applies the portable environment-variable name grammar shared by every target platform.
fn validate_environment_name(name: &str, field: &str) -> Result<(), CompileMcpConfigurationError> {
    let bytes = name.as_bytes();
    let starts_legally =
        matches!(bytes.first(), Some(first) if first.is_ascii_alphabetic() || *first == b'_');
    if !starts_legally
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        return Err(invalid_transport(
            field,
            "environment variable name must match ^[A-Za-z_][A-Za-z0-9_]*$",
        ));
    }
    Ok(())
}

/// Applies the RFC 7230 token grammar to one HTTP header name.
fn validate_header_name(name: &str, field: &str) -> Result<(), CompileMcpConfigurationError> {
    let is_token = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte));
    if !is_token {
        return Err(invalid_transport(
            field,
            "header name must be a valid HTTP token",
        ));
    }
    Ok(())
}

/// Rejects control characters (including CR/LF) in prefix/suffix text bound to any transport
/// position (stdio args, env values, and HTTP headers).
fn validate_bound_text(text: &str, field: &str) -> Result<(), CompileMcpConfigurationError> {
    if text.chars().any(char::is_control) {
        return Err(invalid_transport(
            field,
            "text must not contain control characters",
        ));
    }
    Ok(())
}

/// Builds one transport error with a stable field path.
fn invalid_transport(
    field: impl Into<String>,
    reason: impl Into<String>,
) -> CompileMcpConfigurationError {
    CompileMcpConfigurationError::InvalidTransport {
        field: field.into(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompileConfigurationFileError, CompileMcpConfigurationError, CompiledConfigurationFile,
        McpArgument, McpHttpTransport, McpStdioTransport, McpTransport, McpValueExpression,
        compile_configuration_file,
    };
    use crate::declaration::{CompileDeclarationError, SettingDeclaration, SettingType};
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use url::Url;

    /// Compiles the HTTP shape the Tavily package ships: one required string Setting bound to the
    /// `Authorization` header through a `Bearer ` prefix.
    ///
    /// The Setting ID is `apiKey`, not `api_key`: the existing declaration grammar is
    /// `^[a-z][A-Za-z0-9]{0,63}$`, so an underscore is unrepresentable. Issue 457 follows the
    /// current code contract for identifiers the same way marketplace manifests use `identifier`.
    #[test]
    fn compiles_http_configuration_with_header_setting_reference() {
        let source = br#"{
            "schemaVersion": 1,
            "settings": {
                "apiKey": {
                    "type": "string",
                    "title": "API key",
                    "description": "Key used to authenticate with the MCP server",
                    "required": true
                }
            },
            "transport": {
                "type": "http",
                "url": "https://mcp.tavily.com/mcp",
                "headers": {
                    "Authorization": { "setting": "apiKey", "prefix": "Bearer " }
                }
            }
        }"#;

        let compiled = match compile_configuration_file(source).expect("compile MCP configuration")
        {
            CompiledConfigurationFile::Mcp(compiled) => compiled,
            CompiledConfigurationFile::Settings(_) => panic!("expected the MCP shape"),
        };

        let settings = compiled.settings.expect("settings subset");
        assert_eq!(
            settings.settings,
            vec![SettingDeclaration {
                id: "apiKey".to_string(),
                title: "API key".to_string(),
                description: "Key used to authenticate with the MCP server".to_string(),
                setting_type: SettingType::String,
                required: true,
                order: None,
                default: None,
            }]
        );
        assert_eq!(
            compiled.transport,
            McpTransport::Http(McpHttpTransport {
                url: Url::parse("https://mcp.tavily.com/mcp").expect("endpoint URL"),
                headers: BTreeMap::from([(
                    "Authorization".to_string(),
                    McpValueExpression::Setting {
                        id: "apiKey".to_string(),
                        prefix: "Bearer ".to_string(),
                        suffix: String::new(),
                    },
                )]),
            })
        );
    }

    /// Compiles the stdio shape with literals, Setting references, workspace context, and env.
    #[test]
    fn compiles_stdio_configuration_with_arguments_and_environment() {
        let source = br#"{
            "schemaVersion": 1,
            "settings": {
                "repository": {
                    "type": "string",
                    "title": "Repository",
                    "description": "Repository in owner/name format",
                    "required": true
                },
                "retries": {
                    "type": "number",
                    "title": "Retries",
                    "description": "Retry attempts",
                    "default": 3
                }
            },
            "transport": {
                "type": "stdio",
                "command": "assets/server",
                "args": [
                    "--repository",
                    { "setting": "repository" },
                    "--retries",
                    { "setting": "retries" },
                    "--workspace",
                    { "context": "workspace" },
                    7,
                    true
                ],
                "env": {
                    "SERVER_MODE": "managed",
                    "SERVER_REPOSITORY": { "setting": "repository" }
                }
            }
        }"#;

        let compiled = match compile_configuration_file(source).expect("compile MCP configuration")
        {
            CompiledConfigurationFile::Mcp(compiled) => compiled,
            CompiledConfigurationFile::Settings(_) => panic!("expected the MCP shape"),
        };

        let literal = |text: &str| McpArgument::Value(McpValueExpression::Literal(text.to_owned()));
        let reference = |id: &str| {
            McpArgument::Value(McpValueExpression::Setting {
                id: id.to_owned(),
                prefix: String::new(),
                suffix: String::new(),
            })
        };
        assert_eq!(
            compiled.transport,
            McpTransport::Stdio(McpStdioTransport {
                command: PortableRelativePath::parse("assets/server").expect("command"),
                args: vec![
                    literal("--repository"),
                    reference("repository"),
                    literal("--retries"),
                    reference("retries"),
                    literal("--workspace"),
                    McpArgument::WorkspaceContext,
                    literal("7"),
                    literal("true"),
                ],
                env: BTreeMap::from([
                    (
                        "SERVER_MODE".to_string(),
                        McpValueExpression::Literal("managed".to_string()),
                    ),
                    (
                        "SERVER_REPOSITORY".to_string(),
                        McpValueExpression::Setting {
                            id: "repository".to_string(),
                            prefix: String::new(),
                            suffix: String::new(),
                        },
                    ),
                ]),
            })
        );
    }

    /// An MCP configuration may omit `settings` entirely; the subset is then absent.
    #[test]
    fn compiles_mcp_configuration_without_settings() {
        let source = br#"{
            "schemaVersion": 1,
            "transport": { "type": "http", "url": "https://mcp.example.com/v1" }
        }"#;

        let compiled = match compile_configuration_file(source).expect("compile MCP configuration")
        {
            CompiledConfigurationFile::Mcp(compiled) => compiled,
            CompiledConfigurationFile::Settings(_) => panic!("expected the MCP shape"),
        };

        assert_eq!(compiled.settings, None);
    }

    /// A file without a `transport` member keeps compiling as a Settings-only declaration.
    #[test]
    fn compiles_settings_only_files_through_the_existing_declaration_path() {
        let source = br#"{
            "schemaVersion": 1,
            "settings": {
                "endpoint": {"type":"string","title":"Endpoint","description":"Service URL"}
            }
        }"#;

        assert!(matches!(
            compile_configuration_file(source),
            Ok(CompiledConfigurationFile::Settings(_))
        ));
    }

    /// The reserved spec Setting types fail with the phase-one policy message.
    #[test]
    fn rejects_reserved_setting_types_with_a_targeted_error() {
        for reserved in ["secret", "file", "directory"] {
            let source = format!(
                r#"{{
                    "schemaVersion": 1,
                    "settings": {{
                        "token": {{"type":"{reserved}","title":"Token","description":"Sensitive"}}
                    }},
                    "transport": {{ "type": "http", "url": "https://mcp.example.com/v1" }}
                }}"#
            );

            assert_eq!(
                compile_configuration_file(source.as_bytes()),
                Err(CompileConfigurationFileError::Mcp(
                    CompileMcpConfigurationError::UnsupportedSettingType {
                        setting_id: "token".to_string(),
                        found: reserved.to_string(),
                    }
                )),
            );
        }
    }

    /// Structural rejections: unknown fields, unknown versions, and unknown transports all fail
    /// installation instead of being silently ignored.
    #[test]
    fn rejects_unknown_fields_versions_and_transport_types() {
        let unknown_root = br#"{
            "schemaVersion": 1,
            "transport": { "type": "http", "url": "https://mcp.example.com/v1" },
            "extra": true
        }"#;
        let unknown_version = br#"{
            "schemaVersion": 2,
            "transport": { "type": "http", "url": "https://mcp.example.com/v1" }
        }"#;
        let unknown_transport = br#"{
            "schemaVersion": 1,
            "transport": { "type": "sse", "url": "https://mcp.example.com/v1" }
        }"#;

        assert!(matches!(
            compile_configuration_file(unknown_root),
            Err(CompileConfigurationFileError::Mcp(
                CompileMcpConfigurationError::InvalidStructure(_)
            ))
        ));
        assert_eq!(
            compile_configuration_file(unknown_version),
            Err(CompileConfigurationFileError::Mcp(
                CompileMcpConfigurationError::UnsupportedSchemaVersion(2)
            )),
        );
        assert_eq!(
            compile_configuration_file(unknown_transport),
            Err(CompileConfigurationFileError::Mcp(
                CompileMcpConfigurationError::UnsupportedTransportType("sse".to_string())
            )),
        );
    }

    /// Cross-shape fields are unrepresentable: HTTP rejects `command` and stdio rejects `url`.
    #[test]
    fn rejects_cross_transport_field_combinations() {
        let http_with_command = br#"{
            "schemaVersion": 1,
            "transport": {
                "type": "http",
                "url": "https://mcp.example.com/v1",
                "command": "assets/server"
            }
        }"#;
        let stdio_with_url = br#"{
            "schemaVersion": 1,
            "transport": {
                "type": "stdio",
                "command": "assets/server",
                "url": "https://mcp.example.com/v1"
            }
        }"#;

        for source in [&http_with_command[..], &stdio_with_url[..]] {
            assert!(matches!(
                compile_configuration_file(source),
                Err(CompileConfigurationFileError::Mcp(
                    CompileMcpConfigurationError::InvalidTransport { ref field, .. }
                )) if field == "transport"
            ));
        }
    }

    /// HTTP endpoint policy: HTTPS only, no credentials (userinfo or query), no fragment.
    #[test]
    fn rejects_http_url_policy_violations() {
        let cases = [
            "http://mcp.example.com/v1",
            "https://user:secret@mcp.example.com/v1",
            "https://mcp.example.com/v1#fragment",
            "https://mcp.example.com/mcp?api_key=secret",
            "https://mcp.example.com/mcp?version=1",
            "not a url",
        ];

        for url in cases {
            let source = format!(
                r#"{{ "schemaVersion": 1, "transport": {{ "type": "http", "url": "{url}" }} }}"#
            );
            assert!(
                matches!(
                    compile_configuration_file(source.as_bytes()),
                    Err(CompileConfigurationFileError::Mcp(
                        CompileMcpConfigurationError::InvalidTransport { ref field, .. }
                    )) if field == "transport.url"
                ),
                "{url}"
            );
        }
    }

    /// Header names must be HTTP tokens; header values must be Setting references, not literals.
    #[test]
    fn rejects_invalid_header_names_and_header_literals() {
        let bad_name = br#"{
            "schemaVersion": 1,
            "settings": {
                "apiKey": {"type":"string","title":"API key","description":"Key","required":true}
            },
            "transport": {
                "type": "http",
                "url": "https://mcp.example.com/v1",
                "headers": { "Bad Header": { "setting": "apiKey" } }
            }
        }"#;
        let header_literal = br#"{
            "schemaVersion": 1,
            "transport": {
                "type": "http",
                "url": "https://mcp.example.com/v1",
                "headers": { "Authorization": "Bearer baked-in" }
            }
        }"#;
        let injected_prefix = br#"{
            "schemaVersion": 1,
            "settings": {
                "apiKey": {"type":"string","title":"API key","description":"Key","required":true}
            },
            "transport": {
                "type": "http",
                "url": "https://mcp.example.com/v1",
                "headers": { "Authorization": { "setting": "apiKey", "prefix": "Bearer\r\nInjected: " } }
            }
        }"#;

        for source in [&bad_name[..], &header_literal[..], &injected_prefix[..]] {
            assert!(matches!(
                compile_configuration_file(source),
                Err(CompileConfigurationFileError::Mcp(
                    CompileMcpConfigurationError::InvalidTransport { .. }
                ))
            ));
        }
    }

    /// Every Setting reference must name a declared Setting.
    #[test]
    fn rejects_references_to_undeclared_settings() {
        let source = br#"{
            "schemaVersion": 1,
            "transport": {
                "type": "http",
                "url": "https://mcp.example.com/v1",
                "headers": { "Authorization": { "setting": "apiKey" } }
            }
        }"#;

        assert_eq!(
            compile_configuration_file(source),
            Err(CompileConfigurationFileError::Mcp(
                CompileMcpConfigurationError::InvalidTransport {
                    field: "transport.headers.Authorization".to_string(),
                    reason: "references undeclared Setting `apiKey`".to_string(),
                }
            )),
        );
    }

    /// Command containment: only normalized paths below `assets/` are representable, which
    /// excludes PATH lookup, traversal, absolute paths, and the bare directory itself.
    #[test]
    fn rejects_commands_outside_the_package_assets_directory() {
        let cases = [
            "npx",
            "server",
            "assets",
            "assets/",
            "assets/../orax.toml",
            "/usr/bin/env",
            "C:\\server.exe",
        ];

        for command in cases {
            let source = format!(
                r#"{{
                    "schemaVersion": 1,
                    "transport": {{ "type": "stdio", "command": "{}" }}
                }}"#,
                command.replace('\\', "\\\\")
            );
            assert!(
                matches!(
                    compile_configuration_file(source.as_bytes()),
                    Err(CompileConfigurationFileError::Mcp(
                        CompileMcpConfigurationError::InvalidTransport { ref field, .. }
                    )) if field == "transport.command"
                ),
                "{command}"
            );
        }
    }

    /// Environment variable names follow the portable grammar on every platform.
    #[test]
    fn rejects_invalid_environment_variable_names() {
        for name in ["1BAD", "BAD-NAME", "BAD=NAME", ""] {
            let source = format!(
                r#"{{
                    "schemaVersion": 1,
                    "transport": {{
                        "type": "stdio",
                        "command": "assets/server",
                        "env": {{ "{name}": "value" }}
                    }}
                }}"#
            );
            assert!(
                matches!(
                    compile_configuration_file(source.as_bytes()),
                    Err(CompileConfigurationFileError::Mcp(
                        CompileMcpConfigurationError::InvalidTransport { .. }
                    ))
                ),
                "{name}"
            );
        }
    }

    /// An empty `settings` object is rejected the same way as in a Settings-only declaration.
    #[test]
    fn rejects_an_explicitly_empty_settings_object() {
        let source = br#"{
            "schemaVersion": 1,
            "settings": {},
            "transport": { "type": "http", "url": "https://mcp.example.com/v1" }
        }"#;

        assert_eq!(
            compile_configuration_file(source),
            Err(CompileConfigurationFileError::Mcp(
                CompileMcpConfigurationError::Declaration(CompileDeclarationError::EmptySettings)
            )),
        );
    }
}
