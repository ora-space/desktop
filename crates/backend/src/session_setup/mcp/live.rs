//! In-memory Desired/Active MCP convergence for one Live Session.
//!
//! These states never enter SQLite. Stopped Sessions have no Live MCP state; the next explicit
//! load reads the latest Snapshot instead of replaying a remembered Active revision.

use super::SessionMcpRevision;

/// Live Session MCP convergence state used for prompt admission and refresh.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LiveMcpState {
    /// No provider channel is held, so the Session has no Active MCP revision.
    Inactive,
    Active(SessionMcpRevision),
    RefreshPending {
        active: SessionMcpRevision,
        desired: SessionMcpRevision,
    },
    Refreshing {
        in_flight: SessionMcpRevision,
        newer: Option<SessionMcpRevision>,
    },
    Blocked {
        desired: SessionMcpRevision,
    },
}

/// Observations that move the live MCP state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LiveMcpEvent {
    /// Latest Desired revision was re-read from the catalog.
    DesiredObserved(SessionMcpRevision),
    /// A `session/load` carrying this revision was sent.
    RefreshStarted(SessionMcpRevision),
    /// The in-flight load succeeded for this revision.
    RefreshSucceeded(SessionMcpRevision),
    /// The in-flight load failed. The requested revision is the one that was sent.
    RefreshFailed { requested: SessionMcpRevision },
    /// The provider channel was dropped; the Session is no longer live for MCP.
    Detached,
}

/// Whether a prompt may enter the Agent, or must wait for a refresh.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LiveMcpPromptAdmission {
    Admit,
    RefreshFirst { desired: SessionMcpRevision },
}

impl LiveMcpState {
    /// Applies one observation and returns the next state plus whether an idle refresh is owed.
    pub(crate) fn on_event(&self, event: LiveMcpEvent) -> (Self, bool) {
        match event {
            LiveMcpEvent::Detached => (Self::Inactive, false),
            LiveMcpEvent::DesiredObserved(desired) => self.observe_desired(desired),
            LiveMcpEvent::RefreshStarted(in_flight) => {
                let newer = match self {
                    Self::Refreshing { newer, .. } => newer.clone(),
                    Self::RefreshPending { desired, .. } if *desired != in_flight => {
                        Some(desired.clone())
                    }
                    Self::Blocked { desired } if *desired != in_flight => Some(desired.clone()),
                    _ => None,
                };
                (Self::Refreshing { in_flight, newer }, false)
            }
            LiveMcpEvent::RefreshSucceeded(completed) => self.finish_success(completed),
            LiveMcpEvent::RefreshFailed { requested } => self.finish_failure(requested),
        }
    }

    fn observe_desired(&self, desired: SessionMcpRevision) -> (Self, bool) {
        match self {
            Self::Inactive => (Self::Inactive, false),
            Self::Active(active) if *active == desired => (Self::Active(desired), false),
            Self::Active(active) => (
                Self::RefreshPending {
                    active: active.clone(),
                    desired,
                },
                true,
            ),
            Self::RefreshPending { active, .. } => (
                Self::RefreshPending {
                    active: active.clone(),
                    desired,
                },
                true,
            ),
            Self::Refreshing { in_flight, .. } if *in_flight == desired => (
                Self::Refreshing {
                    in_flight: in_flight.clone(),
                    newer: None,
                },
                false,
            ),
            Self::Refreshing { in_flight, .. } => (
                Self::Refreshing {
                    in_flight: in_flight.clone(),
                    newer: Some(desired),
                },
                false,
            ),
            Self::Blocked { .. } => (Self::Blocked { desired }, true),
        }
    }

    fn finish_success(&self, completed: SessionMcpRevision) -> (Self, bool) {
        let Self::Refreshing { in_flight, newer } = self else {
            return (self.clone(), false);
        };
        if completed != *in_flight {
            return (self.clone(), false);
        }
        match newer {
            Some(newer) if *newer != completed => (
                Self::RefreshPending {
                    active: completed,
                    desired: newer.clone(),
                },
                true,
            ),
            Some(_) | None => (Self::Active(completed), false),
        }
    }

    fn finish_failure(&self, requested: SessionMcpRevision) -> (Self, bool) {
        let Self::Refreshing { in_flight, newer } = self else {
            return (self.clone(), false);
        };
        if requested != *in_flight {
            return (self.clone(), false);
        }
        (
            Self::Blocked {
                desired: newer.clone().unwrap_or(requested),
            },
            false,
        )
    }

    /// Re-reads Desired at prompt admission and decides whether the prompt may enter the Agent.
    pub(crate) fn prompt_admission(&self, desired: &SessionMcpRevision) -> LiveMcpPromptAdmission {
        match self {
            Self::Active(active) if active == desired => LiveMcpPromptAdmission::Admit,
            Self::Inactive
            | Self::Active(_)
            | Self::RefreshPending { .. }
            | Self::Refreshing { .. }
            | Self::Blocked { .. } => LiveMcpPromptAdmission::RefreshFirst {
                desired: desired.clone(),
            },
        }
    }

    /// Whether the Session currently holds an Active revision matching `desired`.
    pub(crate) fn is_current(&self, desired: &SessionMcpRevision) -> bool {
        matches!(self, Self::Active(active) if active == desired)
    }
}
