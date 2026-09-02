//! Resolves compiled MCP bindings against host-owned Setting values.
//!
//! This step still does not know a Session, Agent, or ACP payload. Workspace context stays a
//! distinct argument so the Session layer can substitute the current absolute cwd without this
//! crate learning Agent identity. Setting values are required to produce the bound strings, but
//! they must never appear in this module's errors.

use super::{CompiledMcpConfiguration, McpArgument, McpTransport, McpValueExpression};
use crate::SettingValue;
use ora_utils::path::PortableRelativePath;
use std::collections::BTreeMap;
use thiserror::Error;
use url::Url;

/// One stdio argument after Setting bindings are applied, with workspace context still symbolic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedMcpArgument {
    Literal(String),
    WorkspaceContext,
}

/// One transport after Setting bindings are applied and before ACP mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedMcpTransport {
    Stdio {
        command: PortableRelativePath,
        args: Vec<ResolvedMcpArgument>,
        env: Vec<(String, String)>,
    },
    Http {
        url: Url,
        headers: Vec<(String, String)>,
    },
}

/// Reports a binding that cannot be turned into a runtime string without guessing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResolveMcpBindingError {
    #[error("MCP Setting `{setting_id}` has no effective value")]
    MissingSetting { setting_id: String },
    #[error("MCP Setting `{setting_id}` resolved to illegal runtime text")]
    IllegalRuntimeText { setting_id: String },
}

/// Applies Setting values to one compiled MCP transport without choosing a Session cwd.
///
/// Incomplete configurations are the caller's problem: this function assumes every referenced
/// Setting is present so a later Session setup can fail closed rather than silently omit a
/// member that already qualified for the Effective MCP Set.
pub fn resolve_mcp_transport(
    configuration: &CompiledMcpConfiguration,
    values: &BTreeMap<String, SettingValue>,
) -> Result<ResolvedMcpTransport, ResolveMcpBindingError> {
    match &configuration.transport {
        McpTransport::Stdio(transport) => {
            let mut args = Vec::with_capacity(transport.args.len());
            for argument in &transport.args {
                args.push(match argument {
                    McpArgument::WorkspaceContext => ResolvedMcpArgument::WorkspaceContext,
                    McpArgument::Value(expression) => {
                        ResolvedMcpArgument::Literal(resolve_expression(expression, values)?)
                    }
                });
            }
            let mut env = Vec::with_capacity(transport.env.len());
            for (name, expression) in &transport.env {
                env.push((name.clone(), resolve_expression(expression, values)?));
            }
            Ok(ResolvedMcpTransport::Stdio {
                command: transport.command.clone(),
                args,
                env,
            })
        }
        McpTransport::Http(transport) => {
            let mut headers = Vec::with_capacity(transport.headers.len());
            for (name, expression) in &transport.headers {
                headers.push((name.clone(), resolve_expression(expression, values)?));
            }
            Ok(ResolvedMcpTransport::Http {
                url: transport.url.clone(),
                headers,
            })
        }
    }
}

/// Concatenates prefix, Setting text, and suffix, rejecting control characters introduced at runtime.
fn resolve_expression(
    expression: &McpValueExpression,
    values: &BTreeMap<String, SettingValue>,
) -> Result<String, ResolveMcpBindingError> {
    match expression {
        McpValueExpression::Literal(value) => Ok(value.clone()),
        McpValueExpression::Setting { id, prefix, suffix } => {
            let Some(value) = values.get(id) else {
                return Err(ResolveMcpBindingError::MissingSetting {
                    setting_id: id.clone(),
                });
            };
            let text = format!("{prefix}{}{suffix}", value.as_runtime_text());
            // Compile time already rejected control characters in prefix/suffix and literals.
            // A stored Setting can still introduce them after that check, and those must not
            // become env, argv, or header bytes.
            if text.chars().any(char::is_control) {
                return Err(ResolveMcpBindingError::IllegalRuntimeText {
                    setting_id: id.clone(),
                });
            }
            Ok(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ResolvedMcpArgument, ResolvedMcpTransport, resolve_mcp_transport};
    use crate::SettingValue;
    use crate::mcp::{
        CompiledMcpConfiguration, McpArgument, McpHttpTransport, McpStdioTransport, McpTransport,
        McpValueExpression,
    };
    use ora_utils::path::PortableRelativePath;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use url::Url;

    fn stdio_configuration() -> CompiledMcpConfiguration {
        CompiledMcpConfiguration {
            schema_version: 1,
            settings: None,
            transport: McpTransport::Stdio(McpStdioTransport {
                command: PortableRelativePath::parse("assets/server").expect("command"),
                args: vec![
                    McpArgument::Value(McpValueExpression::Literal(".".to_string())),
                    McpArgument::WorkspaceContext,
                    McpArgument::Value(McpValueExpression::Setting {
                        id: "token".to_string(),
                        prefix: "tok-".to_string(),
                        suffix: String::new(),
                    }),
                ],
                env: BTreeMap::from([(
                    "API_KEY".to_string(),
                    McpValueExpression::Setting {
                        id: "token".to_string(),
                        prefix: String::new(),
                        suffix: String::new(),
                    },
                )]),
            }),
        }
    }

    #[test]
    fn resolves_stdio_bindings_without_substituting_workspace_context() {
        let values = BTreeMap::from([("token".to_string(), SettingValue::String("secret".into()))]);

        assert_eq!(
            resolve_mcp_transport(&stdio_configuration(), &values).expect("resolve"),
            ResolvedMcpTransport::Stdio {
                command: PortableRelativePath::parse("assets/server").expect("command"),
                args: vec![
                    ResolvedMcpArgument::Literal(".".to_string()),
                    ResolvedMcpArgument::WorkspaceContext,
                    ResolvedMcpArgument::Literal("tok-secret".to_string()),
                ],
                env: vec![("API_KEY".to_string(), "secret".to_string())],
            }
        );
    }

    #[test]
    fn resolves_http_header_settings_in_declaration_order() {
        let configuration = CompiledMcpConfiguration {
            schema_version: 1,
            settings: None,
            transport: McpTransport::Http(McpHttpTransport {
                url: Url::parse("https://mcp.example.test/mcp").expect("url"),
                headers: BTreeMap::from([(
                    "Authorization".to_string(),
                    McpValueExpression::Setting {
                        id: "apiKey".to_string(),
                        prefix: "Bearer ".to_string(),
                        suffix: String::new(),
                    },
                )]),
            }),
        };
        let values = BTreeMap::from([("apiKey".to_string(), SettingValue::String("abc".into()))]);

        assert_eq!(
            resolve_mcp_transport(&configuration, &values).expect("resolve"),
            ResolvedMcpTransport::Http {
                url: Url::parse("https://mcp.example.test/mcp").expect("url"),
                headers: vec![("Authorization".to_string(), "Bearer abc".to_string())],
            }
        );
    }

    #[test]
    fn missing_setting_names_the_id_without_the_value() {
        let error = resolve_mcp_transport(&stdio_configuration(), &BTreeMap::new())
            .expect_err("missing setting");
        assert_eq!(
            error.to_string(),
            "MCP Setting `token` has no effective value"
        );
        assert!(!error.to_string().contains("secret"));
    }
}
