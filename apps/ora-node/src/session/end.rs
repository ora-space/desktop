//! How a session execution ended, before the Node identity is attached.

use super::scheduled::ScheduledDeclaration;
use ora_node_protocol::{AgentSessionEndReason, EndSessionReason, PluginId, SessionMcpTransport};

/// Secret-free identifiers of an MCP setup failure destined for the Controller.
pub(super) struct SessionMcpFailure {
    pub(super) plugin_id: Option<Box<PluginId>>,
    pub(super) setting_id: Option<Box<str>>,
    pub(super) transport: Option<SessionMcpTransport>,
}

/// Why a session execution ended, before the Node identity is attached.
pub(super) struct SessionEnd {
    pub(super) reason: AgentSessionEndReason,
    /// A bounded code naming the cause of an `agent_failed` end.
    pub(super) detail: Option<&'static str>,
    /// Present only when the terminal code describes an MCP setup failure.
    pub(super) mcp_failure: Option<Box<SessionMcpFailure>>,
    /// What a scheduled run declares about its business result. Every failure path leaves it
    /// empty: only a run Cloud ended as asked has a completed turn to report.
    pub(super) scheduled: Option<ScheduledDeclaration>,
}

impl SessionEnd {
    /// Captures the first terminal intention before process cleanup starts.
    pub(super) fn terminal(
        &self,
        node: ora_node_protocol::NodeRuntimeIdentity,
    ) -> ora_node_protocol::AgentSessionEnded {
        ora_node_protocol::AgentSessionEnded {
            node,
            reason: self.reason,
            detail: self.detail.map(str::to_owned),
            plugin_id: self.mcp_failure.as_ref().and_then(|f| f.plugin_id.clone()),
            setting_id: self.mcp_failure.as_ref().and_then(|f| f.setting_id.clone()),
            transport: self.mcp_failure.as_ref().and_then(|f| f.transport),
            scheduled_result: self.scheduled.as_ref().and_then(|s| s.result.clone()),
            artifact_manifest: self.scheduled.as_ref().and_then(|s| s.manifest.clone()),
        }
    }
    /// Ends the session because the agent could not be run or kept running.
    pub(super) fn agent_failed(detail: &'static str) -> Self {
        Self {
            reason: AgentSessionEndReason::AgentFailed,
            detail: Some(detail),
            mcp_failure: None,
            scheduled: None,
        }
    }

    /// Ends for MCP setup while retaining only identifiers safe for the Controller.
    pub(super) fn mcp_failed(detail: &'static str, failure: SessionMcpFailure) -> Self {
        Self {
            reason: AgentSessionEndReason::AgentFailed,
            detail: Some(detail),
            mcp_failure: Some(Box::new(failure)),
            scheduled: None,
        }
    }

    /// Ends the session as the command asked.
    pub(super) fn requested(reason: EndSessionReason) -> Self {
        Self {
            reason: match reason {
                EndSessionReason::UserEnded => AgentSessionEndReason::UserEnded,
                EndSessionReason::IdleTimeout => AgentSessionEndReason::IdleTimeout,
                EndSessionReason::Cancelled => AgentSessionEndReason::Cancelled,
            },
            detail: None,
            mcp_failure: None,
            scheduled: None,
        }
    }
}
