//! How a session execution ended, before the Node identity is attached.

use ora_node_protocol::{AgentSessionEndReason, EndSessionReason};

/// Why a session execution ended, before the Node identity is attached.
pub(super) struct SessionEnd {
    pub(super) reason: AgentSessionEndReason,
    /// A bounded code naming the cause of an `agent_failed` end.
    pub(super) detail: Option<&'static str>,
}

impl SessionEnd {
    /// Ends the session because the agent could not be run or kept running.
    pub(super) fn agent_failed(detail: &'static str) -> Self {
        Self {
            reason: AgentSessionEndReason::AgentFailed,
            detail: Some(detail),
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
        }
    }
}
