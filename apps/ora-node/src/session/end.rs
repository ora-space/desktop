//! How a session execution ended, before the Node identity is attached.

use super::scheduled::ScheduledDeclaration;
use ora_node_protocol::{AgentSessionEndReason, EndSessionReason};

/// Why a session execution ended, before the Node identity is attached.
pub(super) struct SessionEnd {
    /// An unconfirmed plugin stop cannot become the terminal evidence authorizing delivery.
    pub(super) stop_confirmed: bool,
    pub(super) reason: AgentSessionEndReason,
    /// A bounded code naming the cause of an `agent_failed` end.
    pub(super) detail: Option<&'static str>,
    /// What a scheduled run declares about its business result. Every failure path leaves it
    /// empty: only a run Cloud ended as asked has a completed turn to report.
    pub(super) scheduled: Option<ScheduledDeclaration>,
}

impl SessionEnd {
    /// Ends the session because the agent could not be run or kept running.
    pub(super) fn agent_failed(detail: &'static str) -> Self {
        Self {
            stop_confirmed: true,
            reason: AgentSessionEndReason::AgentFailed,
            detail: Some(detail),
            scheduled: None,
        }
    }

    /// Ends the session as the command asked.
    pub(super) fn requested(reason: EndSessionReason) -> Self {
        Self {
            stop_confirmed: true,
            reason: match reason {
                EndSessionReason::UserEnded => AgentSessionEndReason::UserEnded,
                EndSessionReason::IdleTimeout => AgentSessionEndReason::IdleTimeout,
                EndSessionReason::Cancelled => AgentSessionEndReason::Cancelled,
            },
            detail: None,
            scheduled: None,
        }
    }
}
