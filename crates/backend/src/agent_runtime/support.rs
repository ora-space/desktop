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
    AgentRef::parse(&agent_ref)
        .map_err(|error| runtime_internal("agent_not_installed", error.to_string()))
}

/// Builds the stable public error for an unknown or deleted Ora session.
pub(super) fn session_not_found(session_id: &str) -> BackendError {
    BackendError::new(
        ErrorClassification::NotFound,
        PublicError::SessionNotFound(EmptyErrorParams {}),
        format!("session not found: {session_id}"),
    )
}

/// Builds the conflict returned when a prompt targets an unloaded logical session.
pub(super) fn session_stopped() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::SessionStopped(EmptyErrorParams {}),
        "session must be loaded before prompting",
    )
}

/// Builds the degraded-mode error while the selected CLI is starting or recovering.
pub(super) fn runtime_unavailable() -> BackendError {
    runtime_internal("agent_runtime_unavailable", "agent runtime is unavailable")
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

/// Hides transport internals behind the backend's stable protocol error.
pub(super) fn map_acp_error(error: ora_acp::AcpError) -> BackendError {
    BackendError::with_source(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        "agent protocol operation failed",
        error,
    )
}

/// Builds an internal runtime error with a caller-selected stable code.
pub(super) fn runtime_internal(code: &'static str, message: impl Into<String>) -> BackendError {
    let (classification, public_error) = match code {
        "agent_not_installed" => (
            ErrorClassification::NotFound,
            PublicError::AgentNotInstalled(EmptyErrorParams {}),
        ),
        "agent_runtime_unavailable" => (
            ErrorClassification::Internal,
            PublicError::AgentRuntimeUnavailable(EmptyErrorParams {}),
        ),
        "session_history_unreadable" => (
            ErrorClassification::Conflict,
            PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
        ),
        _ => (
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
        ),
    };
    BackendError::new(classification, public_error, message)
}

#[cfg(test)]
mod tests {
    use super::{pick_auto_allow_option, runtime_internal};
    use crate::ErrorClassification;
    use agent_client_protocol_schema::v1::{PermissionOption, PermissionOptionKind};
    use ora_contracts::{EmptyErrorParams, PublicError};
    use pretty_assertions::assert_eq;

    /// Keeps unreadable session history on the same typed recovery path as a failed write.
    #[test]
    fn maps_unreadable_session_history_to_degraded_error() {
        let error = runtime_internal(
            "session_history_unreadable",
            "session history could not be read",
        );

        assert_eq!(error.classification(), ErrorClassification::Conflict);
        assert_eq!(
            error.public_error(),
            &PublicError::SessionHistoryDegraded(EmptyErrorParams {})
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
