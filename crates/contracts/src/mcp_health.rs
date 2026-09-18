use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Closed Host MCP health failure codes.
///
/// The family is deliberately separate from Session MCP setup codes (`mcp_setting_missing`,
/// `mcp_http_capability_missing`, …): a probe failure is a runtime observation and never a
/// delivery error. Adding a variant is an external compatibility change and requires a new
/// decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "mcp-health.ts")]
pub enum McpHealthErrorCode {
    /// The stdio process could not be started.
    McpSpawnFailed,
    /// The process exited before the handshake completed.
    McpExitedPrematurely,
    /// `initialize` produced no usable response or a protocol error.
    McpHandshakeFailed,
    /// The hard probe timeout elapsed.
    McpProbeTimeout,
    /// The handshake succeeded but `tools/list` failed.
    McpToolsUnavailable,
    /// DNS, connection, or TLS failure for the HTTP transport.
    McpHttpUnreachable,
    /// HTTP 401 or 403.
    McpHttpUnauthorized,
    /// Another HTTP 4xx or 5xx response.
    McpHttpServerError,
}

impl McpHealthErrorCode {
    /// Stable wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::McpSpawnFailed => "mcp_spawn_failed",
            Self::McpExitedPrematurely => "mcp_exited_prematurely",
            Self::McpHandshakeFailed => "mcp_handshake_failed",
            Self::McpProbeTimeout => "mcp_probe_timeout",
            Self::McpToolsUnavailable => "mcp_tools_unavailable",
            Self::McpHttpUnreachable => "mcp_http_unreachable",
            Self::McpHttpUnauthorized => "mcp_http_unauthorized",
            Self::McpHttpServerError => "mcp_http_server_error",
        }
    }
}

/// Closed reasons an eligible member has no health result yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "mcp-health.ts")]
pub enum McpHealthUnknownReason {
    /// Eligible, but no probe has completed for this identity in this process.
    NotProbed,
    /// The member substitutes workspace context and no real absolute Session cwd is available.
    ContextMissing,
}

/// Transport carried by a health identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "mcp-health.ts")]
pub enum McpHealthTransport {
    Stdio,
    Http,
}

impl McpHealthTransport {
    /// Stable wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }
}

/// Host-observed MCP health outcome for one identity.
///
/// This is Ora's own bounded observation, never the Agent's session-internal state: `Healthy`
/// means the Host completed `initialize` and `tools/list` against this binding product, not that
/// the Agent connected the server or that its tools are visible to the model.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
#[ts(export_to = "mcp-health.ts")]
pub enum McpHealthStatus {
    /// The Host handshake and `tools/list` both succeeded.
    Healthy,
    /// The Host handshake or `tools/list` failed with one stable code.
    Unhealthy { error_code: McpHealthErrorCode },
    /// No result yet, for one of the closed reasons.
    Unknown { reason: McpHealthUnknownReason },
}

/// Secret-free identity of one MCP health cache entry.
///
/// Aligns with the Session MCP member revision — canonical Plugin ID, exact package version,
/// configuration revision, and transport. Members whose arguments substitute
/// `{ "context": "workspace" }` also bind the absolute Session `cwd`; card-view identities leave
/// `cwd` absent so one Workspace's probe can never be presented as another's.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "mcp-health.ts")]
pub struct McpHealthIdentity {
    pub plugin_id: String,
    pub package_version: String,
    #[ts(type = "number")]
    pub configuration_revision: u64,
    pub transport: McpHealthTransport,
    pub cwd: Option<String>,
}

/// One secret-free health entry returned to clients.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "mcp-health.ts")]
pub struct McpHealthEntry {
    pub identity: McpHealthIdentity,
    pub status: McpHealthStatus,
}

/// Lists Host MCP health for currently eligible installed members.
///
/// An absent `cwd` is the plugin-card view; a present absolute `cwd` is the Session-banner view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "mcp-health.ts")]
pub struct ListMcpHealthRequest {
    pub cwd: Option<String>,
}

/// Eligible members and their current in-process health.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "mcp-health.ts")]
pub struct ListMcpHealthResponse {
    pub entries: Vec<McpHealthEntry>,
}

/// Requests one Host MCP health probe for a currently eligible member.
///
/// Absent `cwd` is the card view, where a workspace-context member stays `context_missing`; a
/// present absolute `cwd` allows workspace-context substitution for a Session-scoped identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "mcp-health.ts")]
pub struct ProbeMcpHealthRequest {
    pub plugin_id: String,
    pub cwd: Option<String>,
}

/// Result of one awaited Host MCP health probe.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "mcp-health.ts")]
pub struct ProbeMcpHealthResponse {
    pub entry: McpHealthEntry,
}

/// Exports the MCP health DTO family into one TypeScript module.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    McpHealthErrorCode::export(config)?;
    McpHealthUnknownReason::export(config)?;
    McpHealthTransport::export(config)?;
    McpHealthStatus::export(config)?;
    McpHealthIdentity::export(config)?;
    McpHealthEntry::export(config)?;
    ListMcpHealthRequest::export(config)?;
    ListMcpHealthResponse::export(config)?;
    ProbeMcpHealthRequest::export(config)?;
    ProbeMcpHealthResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ListMcpHealthRequest, ListMcpHealthResponse, McpHealthEntry, McpHealthErrorCode,
        McpHealthIdentity, McpHealthStatus, McpHealthTransport, McpHealthUnknownReason,
        ProbeMcpHealthRequest, ProbeMcpHealthResponse,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies statuses keep the closed code and reason vocabulary on the wire.
    #[test]
    fn serializes_mcp_health_status_wire_shape() {
        assert_eq!(
            serde_json::to_value(McpHealthStatus::Healthy).expect("healthy"),
            json!({ "status": "healthy" }),
        );
        assert_eq!(
            serde_json::to_value(McpHealthStatus::Unhealthy {
                error_code: McpHealthErrorCode::McpHttpUnauthorized,
            })
            .expect("unhealthy"),
            json!({
                "status": "unhealthy",
                "error_code": "mcp_http_unauthorized",
            }),
        );
        assert_eq!(
            serde_json::to_value(McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::ContextMissing,
            })
            .expect("unknown"),
            json!({
                "status": "unknown",
                "reason": "context_missing",
            }),
        );
        assert_eq!(
            serde_json::to_value(McpHealthStatus::Unknown {
                reason: McpHealthUnknownReason::NotProbed,
            })
            .expect("not probed"),
            json!({
                "status": "unknown",
                "reason": "not_probed",
            }),
        );
    }

    /// Verifies every closed error code keeps its exact stable wire token.
    #[test]
    fn serializes_every_error_code_token() {
        let cases = [
            (McpHealthErrorCode::McpSpawnFailed, "mcp_spawn_failed"),
            (
                McpHealthErrorCode::McpExitedPrematurely,
                "mcp_exited_prematurely",
            ),
            (
                McpHealthErrorCode::McpHandshakeFailed,
                "mcp_handshake_failed",
            ),
            (McpHealthErrorCode::McpProbeTimeout, "mcp_probe_timeout"),
            (
                McpHealthErrorCode::McpToolsUnavailable,
                "mcp_tools_unavailable",
            ),
            (
                McpHealthErrorCode::McpHttpUnreachable,
                "mcp_http_unreachable",
            ),
            (
                McpHealthErrorCode::McpHttpUnauthorized,
                "mcp_http_unauthorized",
            ),
            (
                McpHealthErrorCode::McpHttpServerError,
                "mcp_http_server_error",
            ),
        ];
        for (code, token) in cases {
            assert_eq!(code.as_str(), token);
            assert_eq!(
                serde_json::to_value(code).expect("code"),
                json!(token),
                "wire token drifted for {token}"
            );
        }
    }

    /// Verifies the query surface stays secret-free and uses camelCase request fields.
    #[test]
    fn serializes_mcp_health_query_wire_shape() {
        assert_eq!(
            serde_json::to_value(ListMcpHealthRequest { cwd: None }).expect("list request"),
            json!({ "cwd": null }),
        );
        assert_eq!(
            serde_json::to_value(ListMcpHealthResponse {
                entries: vec![McpHealthEntry {
                    identity: McpHealthIdentity {
                        plugin_id: "ora-space/example".to_string(),
                        package_version: "1.2.3".to_string(),
                        configuration_revision: 4,
                        transport: McpHealthTransport::Stdio,
                        cwd: Some("/tmp/ws".to_string()),
                    },
                    status: McpHealthStatus::Healthy,
                }],
            })
            .expect("list response"),
            json!({
                "entries": [{
                    "identity": {
                        "pluginId": "ora-space/example",
                        "packageVersion": "1.2.3",
                        "configurationRevision": 4,
                        "transport": "stdio",
                        "cwd": "/tmp/ws",
                    },
                    "status": { "status": "healthy" },
                }],
            }),
        );
        assert_eq!(
            serde_json::to_value(ProbeMcpHealthRequest {
                plugin_id: "ora-space/example".to_string(),
                cwd: None,
            })
            .expect("probe request"),
            json!({
                "pluginId": "ora-space/example",
                "cwd": null,
            }),
        );
        assert_eq!(
            serde_json::to_value(ProbeMcpHealthResponse {
                entry: McpHealthEntry {
                    identity: McpHealthIdentity {
                        plugin_id: "ora-space/example".to_string(),
                        package_version: "1.0.0".to_string(),
                        configuration_revision: 0,
                        transport: McpHealthTransport::Http,
                        cwd: None,
                    },
                    status: McpHealthStatus::Unhealthy {
                        error_code: McpHealthErrorCode::McpSpawnFailed,
                    },
                },
            })
            .expect("probe response"),
            json!({
                "entry": {
                    "identity": {
                        "pluginId": "ora-space/example",
                        "packageVersion": "1.0.0",
                        "configurationRevision": 0,
                        "transport": "http",
                        "cwd": null,
                    },
                    "status": {
                        "status": "unhealthy",
                        "error_code": "mcp_spawn_failed",
                    },
                },
            }),
        );
    }
}
