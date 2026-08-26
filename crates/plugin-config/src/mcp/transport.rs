//! Compiles the exclusive MCP Transport member and the bound-text rules shared by every position.

use super::{
    CompileMcpConfigurationError, MCP_COMMAND_DIRECTORY, McpArgument, McpHttpTransport,
    McpStdioTransport, McpTransport, McpValueExpression,
};
use ora_utils::path::PortableRelativePath;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use url::Url;

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

/// Dispatches the exclusive transport member on its required `type` discriminator.
pub(super) fn compile_transport(
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
        Value::String(literal) => compile_literal(literal, field),
        // Non-string scalars canonicalize at compile time because the target is always a string.
        Value::Number(literal) => compile_literal(literal.to_string(), field),
        Value::Bool(literal) => compile_literal(literal.to_string(), field),
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
/// into the immutable package.
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

/// Canonicalizes one scalar transport literal after rejecting control characters.
fn compile_literal(
    text: String,
    field: &str,
) -> Result<McpValueExpression, CompileMcpConfigurationError> {
    validate_bound_text(&text, field)?;
    Ok(McpValueExpression::Literal(text))
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

/// Rejects control characters (including CR/LF) in text bound into any transport position:
/// stdio argument and environment literals, and Setting-reference prefix/suffix on args, env,
/// and HTTP headers.
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
