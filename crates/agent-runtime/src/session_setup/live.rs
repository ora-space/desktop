//! In-memory Desired/Active MCP convergence for one Live Session.
//!
//! These states never enter SQLite. Stopped Sessions have no Live MCP state; the next explicit
//! load reads the latest Snapshot instead of replaying a remembered Active revision.

use super::SessionMcpRevision;

/// Live Session MCP convergence state used for prompt admission and refresh.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveMcpState {
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
pub enum LiveMcpEvent {
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
pub enum LiveMcpPromptAdmission {
    Admit,
    RefreshFirst { desired: SessionMcpRevision },
}

impl LiveMcpState {
    /// Applies one observation and returns the next state plus whether an idle refresh is owed.
    pub fn on_event(&self, event: LiveMcpEvent) -> (Self, bool) {
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
            // A failed revision is retried only by a new Desired revision or by prompt admission;
            // a repeated level-triggered observation of the same revision is not a retry input.
            Self::Blocked { desired: blocked } if *blocked == desired => {
                (Self::Blocked { desired }, false)
            }
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
    pub fn prompt_admission(&self, desired: &SessionMcpRevision) -> LiveMcpPromptAdmission {
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
    pub fn is_current(&self, desired: &SessionMcpRevision) -> bool {
        matches!(self, Self::Active(active) if active == desired)
    }
}

#[cfg(test)]
mod tests {
    use super::{LiveMcpEvent, LiveMcpPromptAdmission, LiveMcpState};
    use crate::session_setup::{
        SessionMcpMemberRevision, SessionMcpRevision, SessionMcpTransportKind,
    };
    use ora_domain::PluginId;
    use pretty_assertions::assert_eq;
    use semver::Version;

    /// Builds one member revision under the `ora-space` namespace.
    fn revision(
        name: &str,
        version: Version,
        configuration_revision: u64,
        transport: SessionMcpTransportKind,
    ) -> SessionMcpMemberRevision {
        SessionMcpMemberRevision {
            plugin_id: PluginId::new("ora-space", name).expect("plugin id"),
            package_version: version,
            configuration_revision,
            transport,
        }
    }

    #[test]
    fn live_idle_session_owes_refresh_when_desired_changes() {
        let previous = SessionMcpRevision::new(vec![revision(
            "ready",
            Version::new(1, 0, 0),
            1,
            SessionMcpTransportKind::Stdio,
        )]);
        let next = SessionMcpRevision::new(vec![revision(
            "ready",
            Version::new(1, 0, 0),
            2,
            SessionMcpTransportKind::Stdio,
        )]);
        let (state, refresh) = LiveMcpState::Active(previous.clone())
            .on_event(LiveMcpEvent::DesiredObserved(next.clone()));
        assert_eq!(
            state,
            LiveMcpState::RefreshPending {
                active: previous,
                desired: next.clone(),
            }
        );
        assert!(refresh);
        assert_eq!(
            state.prompt_admission(&next),
            LiveMcpPromptAdmission::RefreshFirst { desired: next }
        );
    }

    #[test]
    fn live_busy_success_cannot_clear_a_newer_pending_revision() {
        let first = SessionMcpRevision::new(vec![revision(
            "ready",
            Version::new(1, 0, 0),
            1,
            SessionMcpTransportKind::Stdio,
        )]);
        let second = SessionMcpRevision::new(vec![revision(
            "ready",
            Version::new(1, 0, 0),
            2,
            SessionMcpTransportKind::Stdio,
        )]);
        let third = SessionMcpRevision::new(vec![revision(
            "ready",
            Version::new(1, 0, 0),
            3,
            SessionMcpTransportKind::Stdio,
        )]);
        let refreshing = LiveMcpState::Refreshing {
            in_flight: first.clone(),
            newer: None,
        };
        let (refreshing, _) = refreshing.on_event(LiveMcpEvent::DesiredObserved(second.clone()));
        let (refreshing, _) = refreshing.on_event(LiveMcpEvent::DesiredObserved(third.clone()));
        let (state, refresh) = refreshing.on_event(LiveMcpEvent::RefreshSucceeded(first.clone()));
        assert_eq!(
            state,
            LiveMcpState::RefreshPending {
                active: first,
                desired: third.clone(),
            }
        );
        assert!(refresh);
        assert!(!state.is_current(&second));
    }

    #[test]
    fn live_refresh_failure_blocks_prompts_until_retry() {
        let desired = SessionMcpRevision::new(vec![revision(
            "ready",
            Version::new(1, 0, 0),
            4,
            SessionMcpTransportKind::Stdio,
        )]);
        let (state, _) = LiveMcpState::Refreshing {
            in_flight: desired.clone(),
            newer: None,
        }
        .on_event(LiveMcpEvent::RefreshFailed {
            requested: desired.clone(),
        });
        assert_eq!(
            state,
            LiveMcpState::Blocked {
                desired: desired.clone()
            }
        );
        assert_eq!(
            state.prompt_admission(&desired),
            LiveMcpPromptAdmission::RefreshFirst { desired }
        );
    }

    #[test]
    fn blocked_session_retries_only_for_a_new_revision() {
        let members = vec![revision(
            "ready",
            Version::new(2, 0, 0),
            1,
            SessionMcpTransportKind::Stdio,
        )];
        let failed = SessionMcpRevision::for_generation(/*generation*/ 1, members.clone());
        let blocked = LiveMcpState::Blocked {
            desired: failed.clone(),
        };

        // Notifications are level-triggered and broadcast, so the failed revision is observed
        // again without any new input; that observation must not spawn another attempt.
        let (state, refresh) = blocked.on_event(LiveMcpEvent::DesiredObserved(failed.clone()));
        assert_eq!((state.clone(), refresh), (blocked.clone(), false));
        assert_eq!(
            state.prompt_admission(&failed),
            LiveMcpPromptAdmission::RefreshFirst {
                desired: failed.clone()
            }
        );

        // Identical members in a later generation are a new retry input.
        let retried = SessionMcpRevision::for_generation(/*generation*/ 2, members);
        let (state, refresh) = blocked.on_event(LiveMcpEvent::DesiredObserved(retried.clone()));
        assert_eq!(
            (state, refresh),
            (LiveMcpState::Blocked { desired: retried }, true)
        );
    }

    #[test]
    fn stopped_sessions_ignore_desired_changes() {
        let desired = SessionMcpRevision::new(vec![revision(
            "ready",
            Version::new(1, 0, 0),
            1,
            SessionMcpTransportKind::Stdio,
        )]);
        let (state, refresh) =
            LiveMcpState::Inactive.on_event(LiveMcpEvent::DesiredObserved(desired));
        assert_eq!(state, LiveMcpState::Inactive);
        assert!(!refresh);
    }
}
