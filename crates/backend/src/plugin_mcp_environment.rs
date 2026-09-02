use ora_effect::Fingerprint;
use ora_effect_mcp::{
    McpAgentFormat, McpOwnershipLedger, configured_environment, materialized_configuration,
    resolve_template,
};
use ora_plugin_config::{ConfigurationCompleteness, ConfigurationService, ConfigurationSummary};
use ora_plugin_lifecycle::ChildProcessEnvironmentProvider;
use ora_plugin_manager::{PluginContribution, PluginManager};
use ora_utils::jsonc::{nested_value, parse_value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const OPENCODE_AGENT_ID: &str = "official/ora-space.opencode";
const CLAUDE_AGENT_ID: &str = "official/ora-space.claude";

/// Injects MCP Setting values only after matching an Agent's exact managed project projection.
#[derive(Clone, Debug)]
pub(crate) struct BackendMcpEnvironmentProvider {
    home_directory: PathBuf,
}

impl BackendMcpEnvironmentProvider {
    /// Binds MCP lookup to Ora's host-owned plugin installation and configuration root.
    pub(crate) fn new(home_directory: PathBuf) -> Self {
        Self { home_directory }
    }
}

impl ChildProcessEnvironmentProvider for BackendMcpEnvironmentProvider {
    fn environment(
        &self,
        agent_plugin_id: &str,
        workspace_root: &Path,
    ) -> Result<BTreeMap<String, String>, String> {
        let Some(target) = target_for_agent(agent_plugin_id) else {
            return Ok(BTreeMap::new());
        };
        environment_for_target(&self.home_directory, workspace_root, target)
    }
}

#[derive(Clone, Copy)]
struct AgentMcpTarget {
    format: McpAgentFormat,
    materialization_format: &'static str,
    config_relative_path: &'static [&'static str],
    sidecar_relative_path: &'static [&'static str],
    object_key: &'static str,
}

/// Maps only the two built-in Agent identities to their fixed project configuration contract.
fn target_for_agent(agent_plugin_id: &str) -> Option<AgentMcpTarget> {
    match agent_plugin_id {
        OPENCODE_AGENT_ID => Some(AgentMcpTarget {
            format: McpAgentFormat::OpenCode,
            materialization_format: "ora/opencode-mcp-config.v1",
            config_relative_path: &[".opencode", "opencode.json"],
            sidecar_relative_path: &[".opencode", ".ora-mcp-managed.json"],
            object_key: "mcp",
        }),
        CLAUDE_AGENT_ID => Some(AgentMcpTarget {
            format: McpAgentFormat::Claude,
            materialization_format: "ora/claude-mcp-config.v1",
            config_relative_path: &[".mcp.json"],
            sidecar_relative_path: &[".claude", ".ora-mcp-managed.json"],
            object_key: "mcpServers",
        }),
        _ => None,
    }
}

/// Resolves all authorized variables without exposing which Setting or value failed validation.
fn environment_for_target(
    home_directory: &Path,
    workspace_root: &Path,
    target: AgentMcpTarget,
) -> Result<BTreeMap<String, String>, String> {
    let canonical_workspace = fs::canonicalize(workspace_root)
        .map_err(|_| "the Agent workspace is unavailable".to_string())?;
    let config_path = contained_existing_path(
        &canonical_workspace,
        target.config_relative_path,
        "MCP configuration",
    )?;
    let sidecar_path = contained_existing_path(
        &canonical_workspace,
        target.sidecar_relative_path,
        "MCP ownership sidecar",
    )?;
    let (Some(config_path), Some(sidecar_path)) = (config_path, sidecar_path) else {
        return Ok(BTreeMap::new());
    };
    let ledger: McpOwnershipLedger = serde_json::from_str(
        &fs::read_to_string(sidecar_path)
            .map_err(|_| "the MCP ownership sidecar is unreadable".to_string())?,
    )
    .map_err(|_| "the MCP ownership sidecar is invalid".to_string())?;
    if ledger.schema_version != 1 || ledger.materialization_format != target.materialization_format
    {
        return Err("the MCP ownership sidecar contract does not match the Agent".to_string());
    }
    let configuration = parse_value(
        &fs::read_to_string(config_path)
            .map_err(|_| "the Agent MCP configuration is unreadable".to_string())?,
    )
    .map_err(|_| "the Agent MCP configuration is invalid".to_string())?;
    let manager = PluginManager::discover(home_directory);
    let service = ConfigurationService::new(home_directory.to_path_buf());
    let mut environment = BTreeMap::new();
    for record in ledger.managed.values() {
        let actual = nested_value(&configuration, target.object_key, &record.server_name)
            .ok_or_else(|| {
                "a managed MCP server is absent from the Agent configuration".to_string()
            })?;
        if fingerprint(actual)? != record.fingerprint {
            return Err("a managed MCP server differs from its ownership proof".to_string());
        }
        let plugin = manager
            .installed_plugins()
            .iter()
            .find(|plugin| plugin.id == record.plugin_id)
            .ok_or_else(|| "a managed MCP plugin is no longer installed".to_string())?;
        let PluginContribution::Mcp(descriptor) = &plugin.contributes else {
            return Err("an ownership proof references a non-MCP plugin".to_string());
        };
        let details = service
            .get(&plugin.id.canonical(), &plugin.package_root)
            .map_err(|_| "MCP configuration is unavailable".to_string())?
            .ok_or_else(|| "MCP configuration is unavailable".to_string())?;
        if details.revision != record.configuration_revision
            || details.summary
                != (ConfigurationSummary::Available {
                    completeness: ConfigurationCompleteness::Complete,
                })
        {
            return Err("MCP configuration changed after project materialization".to_string());
        }
        let template = resolve_template(
            &plugin.id,
            &descriptor.configuration,
            details.revision,
            &plugin.package_root,
        )
        .map_err(|_| "MCP configuration cannot be resolved".to_string())?;
        if template.server_name != record.server_name
            || fingerprint(&materialized_configuration(
                &template,
                target.format,
                &canonical_workspace,
            ))? != record.fingerprint
        {
            return Err("the managed MCP template is no longer current".to_string());
        }
        let bindings = match target.format {
            McpAgentFormat::OpenCode => &template.opencode_environment,
            McpAgentFormat::Claude => &template.claude_environment,
        };
        for (key, value) in configured_environment(&details, bindings)
            .map_err(|_| "MCP environment resolution failed".to_string())?
        {
            if environment.insert(key, value).is_some() {
                return Err("multiple MCP plugins requested the same reserved variable".to_string());
            }
        }
    }
    Ok(environment)
}

/// Resolves an existing workspace child and rejects a symlink or reparse-point escape.
fn contained_existing_path(
    workspace_root: &Path,
    segments: &[&str],
    description: &str,
) -> Result<Option<PathBuf>, String> {
    let candidate = segments
        .iter()
        .fold(workspace_root.to_path_buf(), |path, segment| {
            path.join(segment)
        });
    if !candidate.exists() {
        return Ok(None);
    }
    let canonical =
        fs::canonicalize(candidate).map_err(|_| format!("the {description} is unavailable"))?;
    if !canonical.starts_with(workspace_root) {
        return Err(format!("the {description} escapes the Agent workspace"));
    }
    Ok(Some(canonical))
}

/// Computes the same semantic fingerprint used by the MCP file adapter.
fn fingerprint(value: &serde_json::Value) -> Result<Fingerprint, String> {
    serde_json::to_vec(value)
        .map(|bytes| Fingerprint::sha256(&bytes))
        .map_err(|_| "the MCP configuration could not be fingerprinted".to_string())
}

#[cfg(test)]
mod tests {
    use super::{CLAUDE_AGENT_ID, OPENCODE_AGENT_ID, target_for_agent};
    use pretty_assertions::assert_eq;

    #[test]
    fn only_builtin_agents_receive_an_mcp_target_policy() {
        assert_eq!(
            target_for_agent(OPENCODE_AGENT_ID).map(|target| target.materialization_format),
            Some("ora/opencode-mcp-config.v1")
        );
        assert_eq!(
            target_for_agent(CLAUDE_AGENT_ID).map(|target| target.materialization_format),
            Some("ora/claude-mcp-config.v1")
        );
        assert_eq!(target_for_agent("official/example.agent").is_none(), true);
    }
}
