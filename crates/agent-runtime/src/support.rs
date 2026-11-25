use crate::{ErrorClassification, RuntimeError};
use agent_client_protocol_schema::v1::{
    PermissionOption, PermissionOptionId, PermissionOptionKind, RequestPermissionOutcome,
    RequestPermissionResponse, SelectedPermissionOutcome,
};
use ora_acp::{AcpClient, AcpTransport};
use ora_contracts::{
    AgentRef as ContractAgentRef, RespondToPermissionRequest, RespondToPermissionResponse,
    Session as ContractSession, SessionHistoryState as ContractSessionHistoryState,
    SessionMcpSelection as ContractSessionMcpSelection, SessionStatus as ContractSessionStatus,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_domain::{AgentRef, HistoryState, Session, SessionMcpSelection, SessionStatus};
use std::collections::HashMap;

/// Responds to a pending permission after validating the public request ownership.
pub(super) async fn respond_permission(
    client: &AcpClient<impl AcpTransport>,
    request: RespondToPermissionRequest,
    permissions: &mut HashMap<String, (agent_client_protocol_schema::v1::RequestId, Vec<String>)>,
) -> Result<RespondToPermissionResponse, RuntimeError> {
    let Some((request_id, options)) = permissions.get(&request.permission_request_id) else {
        return Err(RuntimeError::new(
            ErrorClassification::Conflict,
            PublicError::PermissionRequestNotPending(EmptyErrorParams {}),
            "permission request is not pending",
        ));
    };
    if !options.contains(&request.option_id) {
        return Err(RuntimeError::new(
            ErrorClassification::InvalidRequest,
            PublicError::PermissionOptionInvalid(EmptyErrorParams {}),
            "permission option does not belong to this request",
        ));
    }
    let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
        PermissionOptionId::new(request.option_id),
    ));
    client
        .respond(request_id, &RequestPermissionResponse::new(outcome))
        .await
        .map_err(map_acp_error)?;
    permissions.remove(&request.permission_request_id);
    Ok(RespondToPermissionResponse {})
}

/// Picks the option an unattended approval should select from an agent's offered options.
///
/// Prefers `AllowOnce` because a remembered choice is persisted by the agent itself: Claude writes
/// `AllowAlways` into the project's `.claude/settings.local.json`, which on Node is the user's
/// checkout and would ship with the Revision. No person made that "always" choice, so Ora must not
/// leave it behind on either host; answering each similar call again costs only an in-process
/// round trip. Falls back to `AllowAlways` only when the agent offers no one-time option.
pub(super) fn pick_auto_allow_option(options: &[PermissionOption]) -> Option<&PermissionOption> {
    options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::AllowOnce)
        .or_else(|| {
            options
                .iter()
                .find(|option| option.kind == PermissionOptionKind::AllowAlways)
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
        mcp_selection: contract_mcp_selection(session.mcp_selection),
    }
}

/// Projects the persisted MCP authorization as canonical plugin IDs only.
fn contract_mcp_selection(selection: SessionMcpSelection) -> ContractSessionMcpSelection {
    match selection {
        SessionMcpSelection::Automatic => ContractSessionMcpSelection::Automatic,
        SessionMcpSelection::Explicit(ids) => ContractSessionMcpSelection::Explicit(
            ids.iter().map(ora_domain::PluginId::canonical).collect(),
        ),
    }
}

/// Validates a client-supplied agent identity before it is used to select a runtime.
///
/// The transport carries an open string because which agents exist depends on installed plugins,
/// so structural validation happens here. Whether the named agent is actually installed is a
/// separate, later question answered by the supervisor lookup.
pub(super) fn domain_agent_ref(agent_ref: ContractAgentRef) -> Result<AgentRef, RuntimeError> {
    AgentRef::parse(&agent_ref).map_err(|error| agent_not_installed(error.to_string()))
}

/// Builds the stable public error for an unknown or deleted Ora session.
pub(super) fn session_not_found(session_id: &str) -> RuntimeError {
    RuntimeError::new(
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
pub(super) fn session_stopped() -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Conflict,
        PublicError::SessionStopped(EmptyErrorParams {}),
        "session is not attached to a provider",
    )
}

/// Builds the degraded-mode error while the selected CLI is starting or recovering.
pub(super) fn runtime_unavailable() -> RuntimeError {
    runtime_unavailable_because("agent runtime is unavailable")
}

/// Builds the degraded-mode error with the specific reason the runtime could not serve a caller.
///
/// The reason names the agent or supervisor state that refused, which is what distinguishes a
/// runtime that never started from one that is mid-recovery when reading logs after the fact.
pub(super) fn runtime_unavailable_because(context: impl Into<String>) -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Internal,
        PublicError::AgentRuntimeUnavailable(EmptyErrorParams {}),
        context,
    )
}

pub(super) fn runtime_unavailable_with(
    source: impl std::error::Error + Send + Sync + 'static,
) -> RuntimeError {
    RuntimeError::with_source(
        ErrorClassification::Internal,
        PublicError::AgentRuntimeUnavailable(EmptyErrorParams {}),
        "agent CLI runtime is unavailable",
        source,
    )
}

/// Builds the error reported when no installed plugin supplies the requested agent.
pub(super) fn agent_not_installed(context: impl Into<String>) -> RuntimeError {
    RuntimeError::new(
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
pub(super) fn agent_start_failed(detail: impl Into<String>) -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Unprocessable,
        PublicError::AgentStartFailed(EmptyErrorParams {}),
        detail,
    )
}

/// Builds the refusal returned when a live session owes an MCP refresh its agent cannot perform.
///
/// Ora-managed MCP is delivered and refreshed through `session/load`; an agent that does not
/// support it can be sent an empty set but never a changed one. The public error and
/// classification are those Session MCP setup reports for the same missing capability, so the
/// client sees one answer whether the gap was found at setup or at a live refresh.
pub(super) fn mcp_load_unsupported() -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Conflict,
        PublicError::SessionLoadUnsupported(EmptyErrorParams {}),
        "agent does not support session/load required by Ora-managed MCP",
    )
}

/// Reports that the actor cannot accept a second operation while one is in flight.
pub(super) fn session_busy() -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Conflict,
        PublicError::SessionBusy(EmptyErrorParams {}),
        "session already has an active operation",
    )
}

/// Reports that the requested permission no longer belongs to an active prompt.
pub(super) fn permission_not_pending() -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Conflict,
        PublicError::PermissionRequestNotPending(EmptyErrorParams {}),
        "permission request is not pending",
    )
}

/// Builds the error reported when an agent did not answer within a runtime deadline.
///
/// Every deadline in the runtime — process start, ACP initialize, session load, config exchange —
/// collapses onto one public error because the user's response to all of them is to retry. The
/// phase that actually expired stays in `context` for logs, where it distinguishes them.
pub(super) fn agent_timed_out(context: impl Into<String>) -> RuntimeError {
    RuntimeError::new(
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
) -> RuntimeError {
    RuntimeError::with_source(
        ErrorClassification::Unprocessable,
        PublicError::AgentModelDiscoveryFailed(EmptyErrorParams {}),
        "agent model discovery failed",
        source,
    )
}

/// Builds the conflict reported when a session's recorded history cannot be read back.
pub(super) fn session_history_unreadable() -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Conflict,
        PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
        "session history could not be read",
    )
}

/// Builds the internal failure raised when a bounded session queue dropped ordered events.
///
/// Overflow means Ora sized a channel too small or stalled its own consumer, so it stays
/// `InternalError`: the user cannot act on it, and the request ID is what makes it reportable.
pub(super) fn session_event_overflow(context: &'static str) -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        context,
    )
}

/// Builds the internal failure raised when an agent breaks the ACP contract Ora relies on.
///
/// Kept on the same public error as [`map_acp_error`] so a violation detected by Ora's own
/// invariants and one surfaced by the transport read identically to the client.
pub(super) fn protocol_violation(context: &'static str) -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        context,
    )
}

/// Hides transport internals behind the runtime's stable protocol error.
pub(super) fn map_acp_error(error: ora_acp::AcpError) -> RuntimeError {
    RuntimeError::with_source(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        "agent protocol operation failed",
        error,
    )
}

/// Maps a prompt rejection that has one known authentication meaning onto a stable public error.
///
/// Authentication is recognized by the ACP code `-32000` or the two known texts; the text match is
/// kept for Agents that do not use the code. The failure's sanitized, bounded reason is carried so
/// the turn records it, while only the code and `acp_code` reach logs.
pub(super) fn map_prompt_error(error: ora_acp::AcpError) -> RuntimeError {
    match &error {
        ora_acp::AcpError::RequestFailed(request) => {
            let auth = request.code == -32000
                || matches!(
                    request.message.trim(),
                    "Authentication required" | "Not logged in"
                );
            let failure = crate::turn_failure::acp_failure(
                request.code,
                &request.message,
                &request.data,
                auth,
            );
            crate::turn_failure::log_turn_failure("prompt_failed", &failure);
            let runtime_error = if auth {
                RuntimeError::with_source(
                    ErrorClassification::Unprocessable,
                    PublicError::AgentAuthenticationRequired(EmptyErrorParams {}),
                    "agent authentication is required",
                    error,
                )
            } else {
                map_acp_error(error)
            };
            runtime_error.with_turn_failure(failure)
        }
        _ => map_acp_error(error),
    }
}

/// A provider can reject a prompt after dropping its session while keeping the ACP transport
/// alive. Detach that registration so the next turn restores it before sending another prompt.
/// Ordinary request failures (rate limits, model errors, authentication) keep the live channel.
pub(super) fn provider_session_invalid(error: &ora_acp::AcpError) -> bool {
    let ora_acp::AcpError::RequestFailed(request) = error else {
        return false;
    };
    let message = request.message.trim().to_ascii_lowercase();
    matches!(
        message.as_str(),
        "session is closed"
            | "session closed"
            | "session has ended"
            | "session not found"
            | "unknown session"
            | "unknown provider session"
    )
}

/// Builds the refusal returned while a session's history cannot be extended.
pub(super) fn history_degraded() -> RuntimeError {
    RuntimeError::new(
        ErrorClassification::Conflict,
        PublicError::SessionHistoryDegraded(EmptyErrorParams {}),
        "session history could not be recorded and must be resumed first",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        agent_model_discovery_failed, agent_start_failed, agent_timed_out, map_prompt_error,
        pick_auto_allow_option, session_event_overflow, session_history_unreadable,
    };
    use crate::ErrorClassification;
    use agent_client_protocol_schema::v1::{PermissionOption, PermissionOptionKind};
    use ora_contracts::{EmptyErrorParams, PublicError};
    use pretty_assertions::assert_eq;

    /// Cancelling a blocked reply leaves its correlation id available to settlement.
    #[tokio::test]
    async fn a_permission_stays_pending_until_its_reply_is_written() {
        struct BlockedWriter;
        impl ora_acp::AcpTransport for BlockedWriter {
            async fn send(&self, _: serde_json::Value) -> Result<(), ora_acp::AcpError> {
                std::future::pending().await
            }
        }
        let (_input, messages) = tokio::sync::mpsc::unbounded_channel();
        let peer = ora_acp::AcpPeer::spawn(messages, BlockedWriter);
        let mut pending = std::collections::HashMap::from([(
            "permission".to_owned(),
            (
                serde_json::from_str("7").unwrap(),
                vec!["choice".to_owned()],
            ),
        )]);
        {
            let response = super::respond_permission(
                &peer.client,
                ora_contracts::RespondToPermissionRequest {
                    session_id: "session".into(),
                    permission_request_id: "permission".into(),
                    option_id: "choice".into(),
                },
                &mut pending,
            );
            tokio::select! {
                biased;
                _ = response => panic!("the transport has not completed its write"),
                () = std::future::ready(()) => {},
            }
        }
        assert!(pending.contains_key("permission"));
    }

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

    /// Classifies only the observed login refusal and keeps arbitrary request text generic.
    #[test]
    fn classifies_known_authentication_refusal_without_exposing_other_request_errors() {
        let authentication = map_prompt_error(ora_acp::AcpError::RequestFailed(
            ora_acp::AcpRequestError {
                code: -32001,
                message: "Authentication required".to_string(),
                data: None,
            },
        ));
        let not_logged_in = map_prompt_error(ora_acp::AcpError::RequestFailed(
            ora_acp::AcpRequestError {
                code: -32001,
                message: "Not logged in".to_string(),
                data: None,
            },
        ));
        let unknown = map_prompt_error(ora_acp::AcpError::RequestFailed(
            ora_acp::AcpRequestError {
                code: -32001,
                message: "provider diagnostic".to_string(),
                data: None,
            },
        ));

        assert_eq!(
            authentication.public_error(),
            &PublicError::AgentAuthenticationRequired(EmptyErrorParams {})
        );
        assert_eq!(
            not_logged_in.public_error(),
            &PublicError::AgentAuthenticationRequired(EmptyErrorParams {})
        );
        assert_eq!(
            unknown.public_error(),
            &PublicError::InternalError(EmptyErrorParams {})
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

    /// Prefers the one-time option so the agent never persists an approval nobody chose.
    #[test]
    fn prefers_allow_once_over_allow_always() {
        let allow_once =
            PermissionOption::new("allow-once", "Allow once", PermissionOptionKind::AllowOnce);
        let allow_always = PermissionOption::new(
            "allow-always",
            "Allow always",
            PermissionOptionKind::AllowAlways,
        );
        let options = vec![allow_always, allow_once.clone()];

        assert_eq!(pick_auto_allow_option(&options), Some(&allow_once));
    }

    /// Falls back to a remembered allow when the agent offered no one-time option.
    #[test]
    fn falls_back_to_allow_always_without_allow_once() {
        let allow_always = PermissionOption::new(
            "allow-always",
            "Allow always",
            PermissionOptionKind::AllowAlways,
        );
        let reject_once = PermissionOption::new(
            "reject-once",
            "Reject once",
            PermissionOptionKind::RejectOnce,
        );
        let options = vec![reject_once, allow_always.clone()];

        assert_eq!(pick_auto_allow_option(&options), Some(&allow_always));
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
