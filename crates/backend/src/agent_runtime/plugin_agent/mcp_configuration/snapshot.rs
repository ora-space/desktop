//! Builds the complete MCP Configuration Snapshot the Host sends to a plugin.

#![allow(dead_code)] // First production caller is the Agent Target worker in #489.

use ora_effect::{AgentTargetId, ConditionImpact, Generation};
use ora_plugin_runtime::{
    MCP_CONFIGURATION_PROTOCOL_V1, McpConfigurationCapability, McpTransportKind,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fmt::{self, Debug, Formatter};
use std::path::PathBuf;
use thiserror::Error;
use url::Url;

/// One Ready MCP considered for a snapshot before transport filtering.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct DesiredResolvedMcp {
    pub canonical_identity: String,
    pub managed_identity: String,
    pub package_version: String,
    pub source_revision_id: String,
    pub transport: ResolvedMcpTransport,
}

/// Normalized transport carried in a snapshot. Secret values exist only for the plugin call.
#[derive(Clone, Eq, PartialEq)]
pub(crate) enum ResolvedMcpTransport {
    Http {
        url: Url,
        headers: BTreeMap<String, String>,
    },
    Stdio {
        executable: PathBuf,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        working_directory: PathBuf,
    },
}

impl ResolvedMcpTransport {
    /// Returns the capability token used to decide whether this entry is sent to the plugin.
    pub(crate) fn kind(&self) -> McpTransportKind {
        match self {
            Self::Http { .. } => McpTransportKind::Http,
            Self::Stdio { .. } => McpTransportKind::Stdio,
        }
    }
}

impl Debug for ResolvedMcpTransport {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { url, headers } => formatter
                .debug_struct("Http")
                .field("url", &url.host_str().unwrap_or("unknown-host"))
                .field("headers", &redacted_keys(headers.keys()))
                .finish(),
            Self::Stdio {
                executable,
                args,
                env,
                working_directory: _,
            } => formatter
                .debug_struct("Stdio")
                .field("executable", executable)
                .field("args_len", &args.len())
                .field("env", &redacted_keys(env.keys()))
                .finish(),
        }
    }
}

impl Debug for DesiredResolvedMcp {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesiredResolvedMcp")
            .field("canonical_identity", &self.canonical_identity)
            .field("managed_identity", &self.managed_identity)
            .field("package_version", &self.package_version)
            .field("source_revision_id", &self.source_revision_id)
            .field("transport", &self.transport)
            .finish()
    }
}

/// Complete snapshot after unsupported transports have been excluded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedMcpConfiguration {
    pub operation_id: String,
    pub agent_target_id: AgentTargetId,
    pub workspace_root: PathBuf,
    pub generation: Generation,
    pub resolved_mcps: Vec<DesiredResolvedMcp>,
    pub unsupported: Vec<UnsupportedMcp>,
}

/// Target-specific NonBlocking issue for one MCP the Agent cannot materialize.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnsupportedMcp {
    pub managed_identity: String,
    pub transport: McpTransportKind,
    pub impact: ConditionImpact,
    pub code: &'static str,
}

/// Why a snapshot cannot be built for the plugin call.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum SnapshotRequestError {
    #[error("workspace root must be an absolute path")]
    WorkspaceRootNotAbsolute,
    #[error("operation identity must not be empty")]
    EmptyOperationId,
    #[error("agent target identity must not be empty")]
    EmptyAgentTargetId,
}

/// Filters Desired MCP entries by negotiated transports and records NonBlocking skips.
pub(crate) fn prepare_mcp_configuration_snapshot(
    capability: &McpConfigurationCapability,
    operation_id: impl Into<String>,
    agent_target_id: AgentTargetId,
    workspace_root: PathBuf,
    generation: Generation,
    desired: Vec<DesiredResolvedMcp>,
) -> Result<PreparedMcpConfiguration, SnapshotRequestError> {
    let operation_id = operation_id.into();
    if operation_id.is_empty() {
        return Err(SnapshotRequestError::EmptyOperationId);
    }
    if agent_target_id.as_str().is_empty() {
        return Err(SnapshotRequestError::EmptyAgentTargetId);
    }
    if !(workspace_root.is_absolute() || workspace_root.has_root()) {
        // Windows treats `/workspace` as rooted but not absolute; Unix fixtures use that form.
        return Err(SnapshotRequestError::WorkspaceRootNotAbsolute);
    }

    let mut resolved_mcps = Vec::new();
    let mut unsupported = Vec::new();
    for mcp in desired {
        if capability.transports().supports(mcp.transport.kind()) {
            resolved_mcps.push(mcp);
            continue;
        }
        unsupported.push(UnsupportedMcp {
            managed_identity: mcp.managed_identity,
            transport: mcp.transport.kind(),
            impact: ConditionImpact::NonBlocking,
            code: "mcp_unsupported_by_agent",
        });
    }
    Ok(PreparedMcpConfiguration {
        operation_id,
        agent_target_id,
        workspace_root,
        generation,
        resolved_mcps,
        unsupported,
    })
}

/// Serializes the closed snapshot field set the plugin receives.
///
/// The JSON includes header and environment values because the plugin must apply them. Callers
/// must not log this value; diagnostics use `PreparedMcpConfiguration`'s redacted Debug instead.
pub(crate) fn snapshot_request_json(prepared: &PreparedMcpConfiguration) -> Value {
    json!({
        "protocolVersion": MCP_CONFIGURATION_PROTOCOL_V1,
        "operationId": prepared.operation_id,
        "agentTargetId": prepared.agent_target_id.as_str(),
        "workspaceRoot": prepared.workspace_root,
        "generation": prepared.generation.value(),
        "resolvedMcps": prepared
            .resolved_mcps
            .iter()
            .map(|mcp| json!({
                "canonicalIdentity": mcp.canonical_identity,
                "managedIdentity": mcp.managed_identity,
                "packageVersion": mcp.package_version,
                "sourceRevisionId": mcp.source_revision_id,
                "transport": match &mcp.transport {
                    ResolvedMcpTransport::Http { url, headers } => json!({
                        "kind": "http",
                        "url": url.as_str(),
                        "headers": string_map(headers),
                    }),
                    ResolvedMcpTransport::Stdio {
                        executable,
                        args,
                        env,
                        working_directory,
                    } => json!({
                        "kind": "stdio",
                        "executable": executable,
                        "args": args,
                        "env": string_map(env),
                        "workingDirectory": working_directory,
                    }),
                },
            }))
            .collect::<Vec<_>>(),
    })
}

/// Copies a string map in key order so HTTP headers and stdio env stay deterministic.
fn string_map(values: &BTreeMap<String, String>) -> Map<String, Value> {
    values
        .iter()
        .map(|(key, value)| (key.clone(), Value::String(value.clone())))
        .collect()
}

/// Names secret-bearing keys without putting their values into Debug output.
fn redacted_keys<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<String> {
    keys.map(|key| format!("{key}=[REDACTED]")).collect()
}
