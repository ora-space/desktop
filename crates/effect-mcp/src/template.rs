use ora_domain::PluginId;
use ora_effect::{Digest, McpEnvironmentBinding, McpEnvironmentEncoding, McpTemplateDefinition};
use ora_plugin_config::{
    CompiledMcpConfiguration, ConfigurationDetails, McpArgument, McpTransport, McpValueExpression,
    SettingValue,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

const RESERVED_ENV_PREFIX: &str = "ORA_MCP_";
const WORKSPACE_CONTEXT_SENTINEL: &str = "\u{0}ora-workspace-context\u{0}";

/// Selects the Agent-specific projection of one shared MCP template.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpAgentFormat {
    OpenCode,
    Claude,
}

/// Reports an incomplete configuration or a plugin-authored environment collision.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum McpTemplateError {
    #[error("MCP configuration is incomplete")]
    Incomplete,
    #[error("MCP configuration is unavailable")]
    Unavailable,
    #[error("MCP stdio environment name `{0}` is reserved by Ora")]
    ReservedEnvironmentName(String),
    #[error("MCP Setting `{0}` has no effective value")]
    MissingSetting(String),
    #[error("MCP Setting `{0}` is not a string")]
    NonStringSetting(String),
    #[error("MCP Setting value could not be encoded")]
    Encoding,
}

/// Resolves one complete configuration into secret-free OpenCode and Claude templates.
pub fn resolve_template(
    plugin_id: &PluginId,
    configuration: &CompiledMcpConfiguration,
    configuration_revision: u64,
    package_root: &Path,
) -> Result<McpTemplateDefinition, McpTemplateError> {
    let server_name = plugin_id
        .name()
        .rsplit('.')
        .next()
        .unwrap_or(plugin_id.name())
        .to_string();
    let mut opencode_environment = BTreeMap::new();
    let mut claude_environment = BTreeMap::new();
    let (opencode, claude) = match &configuration.transport {
        McpTransport::Http(transport) => {
            let opencode_headers = render_map(
                plugin_id,
                &transport.headers,
                McpEnvironmentEncoding::JsonStringContent,
                "{env:",
                "}",
                &mut opencode_environment,
            );
            let claude_headers = render_map(
                plugin_id,
                &transport.headers,
                McpEnvironmentEncoding::Raw,
                "${",
                "}",
                &mut claude_environment,
            );
            (
                json!({"type":"remote","url":transport.url.as_str(),"headers":opencode_headers}),
                json!({"type":"http","url":transport.url.as_str(),"headers":claude_headers}),
            )
        }
        McpTransport::Stdio(transport) => {
            for name in transport.env.keys() {
                if name.starts_with(RESERVED_ENV_PREFIX) {
                    return Err(McpTemplateError::ReservedEnvironmentName(name.clone()));
                }
            }
            let command = package_root.join(transport.command.to_path_buf());
            let command = command.to_string_lossy().into_owned();
            let opencode_args = render_args(
                plugin_id,
                &transport.args,
                McpEnvironmentEncoding::JsonStringContent,
                "{env:",
                "}",
                &mut opencode_environment,
            );
            let claude_args = render_args(
                plugin_id,
                &transport.args,
                McpEnvironmentEncoding::Raw,
                "${",
                "}",
                &mut claude_environment,
            );
            let opencode_env = render_map(
                plugin_id,
                &transport.env,
                McpEnvironmentEncoding::JsonStringContent,
                "{env:",
                "}",
                &mut opencode_environment,
            );
            let claude_env = render_map(
                plugin_id,
                &transport.env,
                McpEnvironmentEncoding::Raw,
                "${",
                "}",
                &mut claude_environment,
            );
            let mut opencode_command = vec![Value::String(command.clone())];
            opencode_command.extend(opencode_args);
            (
                json!({"type":"local","command":opencode_command,"environment":opencode_env}),
                json!({"type":"stdio","command":command,"args":claude_args,"env":claude_env}),
            )
        }
    };
    Ok(McpTemplateDefinition {
        plugin_id: plugin_id.clone(),
        server_name,
        configuration_revision,
        opencode,
        claude,
        opencode_environment,
        claude_environment,
    })
}

/// Resolves only the environment variables authorized by an exact materialized template.
pub fn configured_environment(
    details: &ConfigurationDetails,
    bindings: &BTreeMap<String, McpEnvironmentBinding>,
) -> Result<BTreeMap<String, String>, McpTemplateError> {
    let values = details
        .settings
        .iter()
        .filter_map(|setting| {
            setting
                .effective_value
                .as_ref()
                .map(|value| (setting.declaration.id.as_str(), value))
        })
        .collect::<BTreeMap<_, _>>();
    let mut environment = BTreeMap::new();
    for binding in bindings.values() {
        let value = values
            .get(binding.setting_id.as_str())
            .ok_or_else(|| McpTemplateError::MissingSetting(binding.setting_id.clone()))?;
        let SettingValue::String(value) = value else {
            return Err(McpTemplateError::NonStringSetting(
                binding.setting_id.clone(),
            ));
        };
        let raw = format!("{}{}{}", binding.prefix, value, binding.suffix);
        let encoded = match binding.encoding {
            McpEnvironmentEncoding::JsonStringContent => serde_json::to_string(&raw)
                .map_err(|_| McpTemplateError::Encoding)?
                .trim_matches('"')
                .to_string(),
            McpEnvironmentEncoding::Raw => raw,
        };
        environment.insert(binding.variable.clone(), encoded);
    }
    Ok(environment)
}

/// Resolves the opaque workspace marker in one Agent-specific secret-free template.
pub fn materialized_configuration(
    definition: &McpTemplateDefinition,
    format: McpAgentFormat,
    workspace_root: &Path,
) -> Value {
    let mut configuration = match format {
        McpAgentFormat::OpenCode => definition.opencode.clone(),
        McpAgentFormat::Claude => definition.claude.clone(),
    };
    replace_workspace_context(&mut configuration, &workspace_root.to_string_lossy());
    configuration
}

fn render_args(
    plugin_id: &PluginId,
    args: &[McpArgument],
    encoding: McpEnvironmentEncoding,
    placeholder_prefix: &str,
    placeholder_suffix: &str,
    bindings: &mut BTreeMap<String, McpEnvironmentBinding>,
) -> Vec<Value> {
    args.iter()
        .map(|argument| match argument {
            McpArgument::WorkspaceContext => Value::String(WORKSPACE_CONTEXT_SENTINEL.to_string()),
            McpArgument::Value(value) => Value::String(render_expression(
                plugin_id,
                value,
                encoding,
                placeholder_prefix,
                placeholder_suffix,
                bindings,
            )),
        })
        .collect()
}

/// Replaces only the renderer's unrepresentable internal marker, preserving literal `.` values.
fn replace_workspace_context(value: &mut Value, workspace: &str) {
    match value {
        Value::String(text) if text == WORKSPACE_CONTEXT_SENTINEL => {
            *text = workspace.to_string();
        }
        Value::Array(values) => {
            for value in values {
                replace_workspace_context(value, workspace);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                replace_workspace_context(value, workspace);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn render_map(
    plugin_id: &PluginId,
    values: &BTreeMap<String, McpValueExpression>,
    encoding: McpEnvironmentEncoding,
    placeholder_prefix: &str,
    placeholder_suffix: &str,
    bindings: &mut BTreeMap<String, McpEnvironmentBinding>,
) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                render_expression(
                    plugin_id,
                    value,
                    encoding,
                    placeholder_prefix,
                    placeholder_suffix,
                    bindings,
                ),
            )
        })
        .collect()
}

fn render_expression(
    plugin_id: &PluginId,
    expression: &McpValueExpression,
    encoding: McpEnvironmentEncoding,
    placeholder_prefix: &str,
    placeholder_suffix: &str,
    bindings: &mut BTreeMap<String, McpEnvironmentBinding>,
) -> String {
    match expression {
        McpValueExpression::Literal(value) => value.clone(),
        McpValueExpression::Setting { id, prefix, suffix } => {
            let seed = format!(
                "{}\0{id}\0{prefix}\0{suffix}\0{encoding:?}",
                plugin_id.canonical()
            );
            let digest = Digest::sha256(seed.as_bytes());
            let variable = format!(
                "{RESERVED_ENV_PREFIX}{}",
                digest.as_str()["sha256:".len()..][..24].to_ascii_uppercase()
            );
            bindings.insert(
                variable.clone(),
                McpEnvironmentBinding {
                    variable: variable.clone(),
                    setting_id: id.clone(),
                    prefix: prefix.clone(),
                    suffix: suffix.clone(),
                    encoding,
                },
            );
            format!("{placeholder_prefix}{variable}{placeholder_suffix}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        McpAgentFormat, configured_environment, materialized_configuration, resolve_template,
    };
    use ora_domain::PluginId;
    use ora_plugin_config::{
        CompiledConfigurationFile, ConfigurationCompleteness, ConfigurationDetails,
        ConfigurationSummary, EffectiveValueSource, SettingDetails, SettingValue,
        compile_configuration_file,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::path::Path;

    /// Resolves a complete fixture into its compiled MCP and effective Setting details.
    fn configured_fixture(
        source: &str,
        secret: &str,
    ) -> (
        ora_plugin_config::CompiledMcpConfiguration,
        ConfigurationDetails,
    ) {
        let CompiledConfigurationFile::Mcp(configuration) =
            compile_configuration_file(source.as_bytes()).expect("compile MCP fixture")
        else {
            panic!("fixture must compile as MCP");
        };
        let declaration = configuration.settings.clone().expect("fixture settings");
        let settings = declaration
            .settings
            .iter()
            .cloned()
            .map(|declaration| SettingDetails {
                declaration,
                stored_value: Some(SettingValue::String(secret.to_string())),
                effective_value: Some(SettingValue::String(secret.to_string())),
                source: EffectiveValueSource::Stored,
                value_error_code: None,
            })
            .collect();
        let details = ConfigurationDetails {
            declaration,
            settings,
            revision: 7,
            summary: ConfigurationSummary::Available {
                completeness: ConfigurationCompleteness::Complete,
            },
        };
        (configuration, details)
    }

    #[test]
    fn http_templates_are_secret_free_while_each_agent_receives_its_required_encoding() {
        let (configuration, details) = configured_fixture(
            r#"{
              "schemaVersion": 1,
              "settings": {"apiKey":{"title":"API key","description":"Key","type":"string","required":true}},
              "transport": {"type":"http","url":"https://mcp.example.test","headers":{"Authorization":{"setting":"apiKey","prefix":"Bearer "}}}
            }"#,
            "quote\"and\\slash",
        );
        let template = resolve_template(
            &PluginId::parse("official/example.mcp").expect("plugin id"),
            &configuration,
            7,
            Path::new("package"),
        )
        .expect("resolve template");
        let serialized = serde_json::to_string(&template).expect("serialize template");
        assert_eq!(serialized.contains("quote"), false);
        assert_eq!(
            configured_environment(&details, &template.opencode_environment)
                .expect("OpenCode environment")
                .values()
                .next(),
            Some(&"Bearer quote\\\"and\\\\slash".to_string())
        );
        assert_eq!(
            configured_environment(&details, &template.claude_environment)
                .expect("Claude environment")
                .values()
                .next(),
            Some(&"Bearer quote\"and\\slash".to_string())
        );
    }

    #[test]
    fn workspace_context_does_not_replace_a_literal_dot_argument() {
        let source = r#"{
          "schemaVersion":1,
          "transport":{"type":"stdio","command":"assets/server","args":[".",{"context":"workspace"}]}
        }"#;
        let CompiledConfigurationFile::Mcp(configuration) =
            compile_configuration_file(source.as_bytes()).expect("compile MCP fixture")
        else {
            panic!("fixture must compile as MCP");
        };
        let template = resolve_template(
            &PluginId::parse("official/example.stdio").expect("plugin id"),
            &configuration,
            0,
            Path::new("package"),
        )
        .expect("resolve template");

        assert_eq!(
            materialized_configuration(&template, McpAgentFormat::Claude, Path::new("workspace")),
            json!({
                "type":"stdio",
                "command":Path::new("package").join("assets").join("server").to_string_lossy(),
                "args":[".","workspace"],
                "env":{}
            })
        );
    }
}
