use super::RuntimeActor;
use super::events::{drain_queued_prompt_events, settle_cancelled_prompt};
use super::limits::CANCELLATION_GRACE;
use super::prompt_liveness::PromptLiveness;
use super::routing::SessionChannel;
use super::{agent_timed_out, map_acp_error};
use crate::BackendError;
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::SessionId as AcpSessionId;
use agent_client_protocol_schema::v1::{PromptRequest, PromptResponse, RequestId, StopReason};
use ora_contracts::PromptSessionEvent;
use ora_logging::{ora_debug, ora_warn};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// What the prompt loop does after its meaningful-activity window expired.
pub(super) enum StalledPrompt {
    /// The prompt was re-sent on the same provider session; keep streaming this request.
    Resent(ora_acp::PendingSessionRequest<PromptResponse>),
    /// No retry is possible or the re-send failed; the loop ends the turn with this error.
    Failed(BackendError),
}

/// Cancels a stalled attempt and, while the schedule allows, re-sends the same prompt.
///
/// The order matters: `session/cancel` first, then the stalled request's response
/// fence, and only then a new `session/prompt`. ACP updates carry no request id, so
/// the fence is the only proof that nothing the abandoned attempt still had in
/// flight can be mistaken for the retry's output. `session/close` is deliberately
/// not part of a retry: it would drop the provider session and force the next send
/// through load or handoff, which is the expensive path the retry exists to avoid.
pub(super) async fn retry_stalled_prompt(
    actor: &mut RuntimeActor,
    channel: &mut SessionChannel,
    pending: ora_acp::PendingSessionRequest<PromptResponse>,
    events: &mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
    liveness: &mut PromptLiveness,
    permissions: &mut HashMap<String, (RequestId, Vec<String>)>,
    request: &PromptRequest,
) -> StalledPrompt {
    ora_warn!(session_id = %actor.session.id, "prompt inactive; cancelling stalled attempt");
    let client = channel.connection.client.clone();
    actor.cancel(&client, permissions).await;
    let settled = timeout(
        CANCELLATION_GRACE,
        settle_cancelled_prompt(actor, channel, &client, pending, events),
    )
    .await;
    // Only a cancellation the agent itself confirmed leaves the provider session in a
    // state a new prompt can share. A response with another stop reason means the
    // agent finished the turn at the boundary, and a late or missing fence means it
    // is wedged; neither is a stall worth re-sending into, so both fail the prompt
    // exactly as an unretried timeout did.
    let retry = match settled {
        Ok(Some(Ok(PromptResponse {
            stop_reason: StopReason::Cancelled,
            ..
        }))) => liveness.next_attempt(),
        Ok(Some(Ok(_) | Err(_))) | Ok(None) | Err(_) => None,
    };
    let Some(retry) = retry else {
        if !matches!(settled, Ok(Some(_))) {
            drain_queued_prompt_events(actor, channel, &client, events).await;
        }
        return StalledPrompt::Failed(agent_timed_out("agent prompt made no progress"));
    };
    ora_warn!(session_id = %actor.session.id, retry = retry.retry, max_retries = retry.max_retries, "prompt inactive; re-sending prompt");
    // The client hears about the retry before the re-sent prompt can produce anything,
    // so it can attribute what follows to the retry. Only the owning stream is told:
    // followers see one continuous turn either way.
    if events
        .try_send(Ok(PromptSessionEvent::Retrying {
            retry: retry.retry,
            max_retries: retry.max_retries,
        }))
        .is_err()
    {
        return StalledPrompt::Failed(agent_timed_out("agent prompt made no progress"));
    }
    // The stalled attempt's permission requests were answered by the cancel; a late
    // client response to one of them must not reach the retry.
    permissions.clear();
    // Same provider session and the same blocks: the transcript handoff, if this prompt
    // carried one, was settled by the first accepted send and is not owed again. The
    // prompt is also already recorded, so history keeps a single user turn.
    ora_debug!(session_id = %actor.session.id, retry = retry.retry, "session/prompt re-sent");
    match client
        .start_session_request::<_, PromptResponse>(
            AcpSessionId::new(actor.provider_session_id().to_string()),
            AGENT_METHOD_NAMES.session_prompt,
            request,
        )
        .await
    {
        Ok(pending) => StalledPrompt::Resent(pending),
        Err(error) => StalledPrompt::Failed(map_acp_error(error)),
    }
}
