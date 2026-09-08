use super::connection::AgentAcpClient;
use crate::{BackendError, ErrorClassification};
use agent_client_protocol_schema::v1::{
    PermissionOption, PermissionOptionId, PermissionOptionKind, RequestPermissionOutcome,
    RequestPermissionResponse, SelectedPermissionOutcome,
};
use ora_contracts::{
    AgentRef as ContractAgentRef, RespondToPermissionRequest, RespondToPermissionResponse,
    Session as ContractSession, SessionHistoryState as ContractSessionHistoryState,
    SessionStatus as ContractSessionStatus,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_domain::{AgentRef, HistoryState, Session, SessionStatus};
use std::collections::HashMap;

/// Responds to a pending permission after validating the public request ownership.
pub(super) async fn respond_permission(
    client: &AgentAcpClient,
    request: RespondToPermissionRequest,
    permissions: &mut HashMap<String, (agent_client_protocol_schema::v1::RequestId, Vec<String>)>,
) -> Result<RespondToPermissionResponse, BackendError> {
    let Some((request_id, options)) = permissions.remove(&request.permission_request_id) else {
        return Err(BackendError::new(
            ErrorClassification::Conflict,
            PublicError::PermissionRequestNotPending(EmptyErrorParams {}),
            "permission request is not pending",
        ));
    };
    if !options.contains(&request.option_id) {
        permissions.insert(request.permission_request_id, (request_id, options));
        return Err(BackendError::new(
            ErrorClassification::InvalidRequest,
            PublicError::PermissionOptionInvalid(EmptyErrorParams {}),
            "permission option does not belong to this request",
        ));
    }
    let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
        PermissionOptionId::new(request.option_id),
    ));
    client
        .respond(&request_id, &RequestPermissionResponse::new(outcome))
        .await
        .map_err(map_acp_error)?;
    Ok(RespondToPermissionResponse {})
}

/// Picks the option an unattended approval should select from an agent's offered options.
///
/// Prefers `AllowAlways` so later, similar calls in the same turn are also covered; falls back
/// to `AllowOnce` when the agent did not offer a remembered-choice option.
pub(super) fn pick_auto_allow_option(options: &[PermissionOption]) -> Option<&PermissionOption> {
    options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::AllowAlways)
        .or_else(|| {
            options
                .iter()
                .find(|option| option.kind == PermissionOptionKind::AllowOnce)
        })
}

/// Maps a private domain session into its frontend-safe view.
pub(super) fn contract_session(session: Session) -> ContractSession {
    ContractSession {
        id: session.id.to_string(),
        workspace_id: session.workspace_id.to_string(),
        title: session.title.map(|title| title.as_str().to_owned()),
        agent_ref: session.agent_ref.into(),
        status: match session.status {
            SessionStatus::Running => ContractSessionStatus::Running,
            SessionStatus::Stopped => ContractSessionStatus::Stopped,
        },
        history_state: match session.history_state {
            HistoryState::Writable => ContractSessionHistoryState::Writable,
            HistoryState::Degraded { reason } => ContractSessionHistoryState::Degraded { reason },
        },
    }
}

/// Validates a client-supplied agent identity before it is used to select a runtime.
///
/// The transport carries an open string because which agents exist depends on installed plugins,
/// so structural validation happens here. Whether the named agent is actually installed is a
/// separate, later question answered by the supervisor lookup.
pub(super) fn domain_agent_ref(agent_ref: ContractAgentRef) -> Result<AgentRef, BackendError> {
    AgentRef::parse(&agent_ref).map_err(|error| agent_not_installed(error.to_string()))
}

/// Builds the stable public error for an unknown or deleted Ora session.
pub(super) fn session_not_found(session_id: &str) -> BackendError {
    BackendError::new(
        ErrorClassification::NotFound,
        PublicError::SessionNotFound(EmptyErrorParams {}),
        format!("session not found: {session_id}"),
    )
}

/// Builds the conflict returned when a live session lost the provider channel it was using.
///
/// No longer a prompt precondition: sending attaches on its own. It survives because a live MCP
/// refresh can find the channel already gone, and that is the same "this session is not attached
/// to anything right now" answer the caller has always been given.
pub(super) fn session_stopped() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::SessionStopped(EmptyErrorParams {}),
        "session is not attached to a provider",
    )
}

/// Builds the degraded-mode error while the selected CLI is starting or recovering.
pub(super) fn runtime_unavailable() -> BackendError {
    runtime_unavailable_because("agent runtime is unavailable")
}

/// Builds the degraded-mode error with the specific reason the runtime could not serve a caller.
///
/// The reason names the agent or supervisor state that refused, which is what distinguishes a
/// runtime that never started from one that is mid-recovery when reading logs after the fact.
pub(super) fn runtime_unavailable_because(context: impl Into<String>) -> BackendError {
    BackendError::new(
        ErrorClassification::Internal,
        PublicError::AgentRuntimeUnavailable(EmptyErrorParams {}),
        context,
    )
}

pub(super) fn runtime_unavailable_with(
    source: impl std::error::Error + Send + Sync + 'static,
) -> BackendError {
    BackendError::with_source(
        ErrorClassification::Internal,
        PublicError::AgentRuntimeUnavailable(EmptyErrorParams {}),
        "agent CLI runtime is unavailable",
        source,
    )
}

/// Builds the error reported when no installed plugin supplies the requested agent.
pub(super) fn agent_not_installed(context: impl Into<String>) -> BackendError {
    BackendError::new(
        ErrorClassification::NotFound,
        PublicError::AgentNotInstalled(EmptyErrorParams {}),
        context,
    )
}

/// Builds the error reported when an installed agent exists but its process could not be started.
///
/// `detail` is authored by the plugin behind the agent, so it stays in the private context that
/// reaches logs only. Routing it into the public error would render unvalidated third-party text
/// in the user's own language-selected UI, so `AgentStartFailed` carries no parameters.
///
/// Classified `Unprocessable` rather than `Internal`: a CLI that is missing, unusable, or
/// misconfigured on this machine is an expected local condition the user can act on, not an Ora
/// defect worth an ERROR-level completion record and a request ID to report.
pub(super) fn agent_start_failed(detail: impl Into<String>) -> BackendError {
    BackendError::new(
        ErrorClassification::Unprocessable,
        PublicError::AgentStartFailed(EmptyErrorParams {}),
        detail,
    )
}

/// Builds the error reported when an agent did not answer within a runtime deadline.
///
/// Every deadline in the runtime — process start, ACP initialize, session load, config exchange —
/// collapses onto one public error because the user's response to all of them is to retry. The
/// phase that actually expired stays in `context` for logs, where it distinguishes them.
pub(super) fn agent_timed_out(context: &'static str) -> BackendError {
    BackendError::new(
        ErrorClassification::Unprocessable,
        PublicError::AgentTimedOut(EmptyErrorParams {}),
        context,
    )
}

/// Builds the error reported when an agent could not enumerate the models it offers.
///
/// Model discovery runs against the agent's own process, so a failure is that agent's condition
/// rather than an Ora defect; the underlying transport or plugin error is kept as the source.
pub(super) fn agent_model_discovery_failed(
    source: impl std::error::Error + Send + Sync + 'static,
) -> BackendError {
    BackendError::with_source(
        ErrorClassification::Unprocessable,
        PublicError::AgentModelDiscoveryFailed(EmptyErrorParams {}),
        "agent model discovery failed",
        source,
    )
}

/// Builds the conflict reported when a session's recorded history cannot be read back.
pub(super) fn session_history_unreadable() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
        "session history could not be read",
    )
}

/// Builds the internal failure raised when a bounded session queue dropped ordered events.
///
/// Overflow means Ora sized a channel too small or stalled its own consumer, so it stays
/// `InternalError`: the user cannot act on it, and the request ID is what makes it reportable.
pub(super) fn session_event_overflow(context: &'static str) -> BackendError {
    BackendError::new(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        context,
    )
}

/// Builds the internal failure raised when an agent breaks the ACP contract Ora relies on.
///
/// Kept on the same public error as [`map_acp_error`] so a violation detected by Ora's own
/// invariants and one surfaced by the transport read identically to the client.
pub(super) fn protocol_violation(context: &'static str) -> BackendError {
    BackendError::new(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        context,
    )
}

/// Hides transport internals behind the backend's stable protocol error.
pub(super) fn map_acp_error(error: ora_acp::AcpError) -> BackendError {
    BackendError::with_source(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        "agent protocol operation failed",
        error,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        agent_model_discovery_failed, agent_start_failed, agent_timed_out, pick_auto_allow_option,
        session_event_overflow, session_history_unreadable,
    };
    use crate::ErrorClassification;
    use agent_client_protocol_schema::v1::{PermissionOption, PermissionOptionKind};
    use ora_contracts::{EmptyErrorParams, PublicError};
    use pretty_assertions::assert_eq;

    /// Keeps unreadable session history on the same typed recovery path as a failed write.
    #[test]
    fn maps_unreadable_session_history_to_degraded_error() {
        let error = session_history_unreadable();

        assert_eq!(error.classification(), ErrorClassification::Conflict);
        assert_eq!(
            error.public_error(),
            &PublicError::SessionHistoryDegraded(EmptyErrorParams {})
        );
    }

    /// Keeps a plugin-authored startup detail out of the contract the frontend renders.
    ///
    /// The detail is unvalidated third-party text, so it must reach logs only. A parameter-carrying
    /// public error here would put it in the user's toast in whichever language the plugin wrote.
    #[test]
    fn reports_agent_start_failure_without_leaking_plugin_detail() {
        let error = agent_start_failed("plugin said: /usr/bin/foo is not executable");

        assert_eq!(error.classification(), ErrorClassification::Unprocessable);
        assert_eq!(
            error.public_error(),
            &PublicError::AgentStartFailed(EmptyErrorParams {})
        );
    }

    /// Retains the agent transport failure that explains why model enumeration failed.
    #[test]
    fn preserves_model_discovery_source() {
        let error = agent_model_discovery_failed(std::io::Error::other("plugin channel closed"));

        assert_eq!(error.classification(), ErrorClassification::Unprocessable);
        assert_eq!(
            error.public_error(),
            &PublicError::AgentModelDiscoveryFailed(EmptyErrorParams {})
        );
        assert_eq!(
            std::error::Error::source(&error).map(ToString::to_string),
            Some("plugin channel closed".to_string())
        );
    }

    /// Keeps every runtime deadline on one retryable public error instead of four near-identical ones.
    #[test]
    fn collapses_every_runtime_deadline_onto_one_public_error() {
        let phases = [
            "agent plugin start timed out",
            "agent CLI session load timed out",
            "agent CLI session creation timed out",
            "agent initialization timed out",
            "agent configuration timed out",
        ];

        for phase in phases {
            let error = agent_timed_out(phase);

            assert_eq!(error.classification(), ErrorClassification::Unprocessable);
            assert_eq!(
                error.public_error(),
                &PublicError::AgentTimedOut(EmptyErrorParams {})
            );
        }
    }

    /// Keeps queue overflow reportable: it is Ora's own defect, so the request ID must stay useful.
    #[test]
    fn reports_queue_overflow_as_an_internal_defect() {
        let error = session_event_overflow("session event queue overflowed");

        assert_eq!(error.classification(), ErrorClassification::Internal);
        assert_eq!(
            error.public_error(),
            &PublicError::InternalError(EmptyErrorParams {})
        );
    }

    /// Prefers the remembered-choice option so later, similar calls stay unattended too.
    #[test]
    fn prefers_allow_always_over_allow_once() {
        let allow_once =
            PermissionOption::new("allow-once", "Allow once", PermissionOptionKind::AllowOnce);
        let allow_always = PermissionOption::new(
            "allow-always",
            "Allow always",
            PermissionOptionKind::AllowAlways,
        );
        let options = vec![allow_once, allow_always.clone()];

        assert_eq!(pick_auto_allow_option(&options), Some(&allow_always));
    }

    /// Falls back to a one-time allow when the agent offered no remembered-choice option.
    #[test]
    fn falls_back_to_allow_once_without_allow_always() {
        let allow_once =
            PermissionOption::new("allow-once", "Allow once", PermissionOptionKind::AllowOnce);
        let reject_once = PermissionOption::new(
            "reject-once",
            "Reject once",
            PermissionOptionKind::RejectOnce,
        );
        let options = vec![reject_once, allow_once.clone()];

        assert_eq!(pick_auto_allow_option(&options), Some(&allow_once));
    }

    /// Reports no selectable option when the agent offered only rejections.
    #[test]
    fn returns_none_without_any_allow_option() {
        let reject_once = PermissionOption::new(
            "reject-once",
            "Reject once",
            PermissionOptionKind::RejectOnce,
        );
        let reject_always = PermissionOption::new(
            "reject-always",
            "Reject always",
            PermissionOptionKind::RejectAlways,
        );
        let options = vec![reject_once, reject_always];

        assert_eq!(pick_auto_allow_option(&options), None);
    }
}
