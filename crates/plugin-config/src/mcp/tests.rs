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

    let compiled = match compile_configuration_file(source).expect("compile MCP configuration") {
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

    let compiled = match compile_configuration_file(source).expect("compile MCP configuration") {
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

    let compiled = match compile_configuration_file(source).expect("compile MCP configuration") {
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

/// Stdio argument and environment literals share the same bound-text rule as header prefix/suffix
/// so a package cannot smuggle CR/LF into later spawn or env application.
#[test]
fn rejects_control_characters_in_stdio_literals() {
    let arg_crlf = br#"{
            "schemaVersion": 1,
            "transport": {
                "type": "stdio",
                "command": "assets/server",
                "args": ["--flag\r\n--injected"]
            }
        }"#;
    let env_crlf = br#"{
            "schemaVersion": 1,
            "transport": {
                "type": "stdio",
                "command": "assets/server",
                "env": { "MODE": "ok\r\nINJECTED=1" }
            }
        }"#;

    assert_eq!(
        compile_configuration_file(arg_crlf),
        Err(CompileConfigurationFileError::Mcp(
            CompileMcpConfigurationError::InvalidTransport {
                field: "transport.args[0]".to_string(),
                reason: "text must not contain control characters".to_string(),
            }
        )),
    );
    assert_eq!(
        compile_configuration_file(env_crlf),
        Err(CompileConfigurationFileError::Mcp(
            CompileMcpConfigurationError::InvalidTransport {
                field: "transport.env.MODE".to_string(),
                reason: "text must not contain control characters".to_string(),
            }
        )),
    );
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
