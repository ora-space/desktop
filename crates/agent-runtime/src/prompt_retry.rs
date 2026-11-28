use super::RuntimeActor;
use super::agent_timed_out;
use super::events::{drain_queued_prompt_events, settle_cancelled_prompt};
use super::limits::CANCELLATION_GRACE;
use super::prompt_liveness::PromptLiveness;
use super::routing::SessionChannel;
use super::support::map_prompt_error;
use crate::AgentRuntimeHost;
use crate::RuntimeError;
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::SessionId as AcpSessionId;
use agent_client_protocol_schema::v1::{PromptRequest, PromptResponse, RequestId, StopReason};
use ora_contracts::PromptSessionEvent;
use ora_domain::PromptInactivityPolicy;
use ora_history::{TurnFailure, TurnFailureAnswer};
use ora_logging::{ora_debug, ora_warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Instant;

/// What the prompt loop does after its meaningful-activity window expired.
pub(super) enum StalledPrompt {
    /// The prompt was re-sent on the same provider session; keep streaming this request.
    Resent(ora_acp::PendingSessionRequest<PromptResponse>),
    /// No retry is possible or the re-send failed; the loop ends the turn with this error.
    Failed(RuntimeError),
    /// A turn cancel merged into this stop: never resend. `reusable` says whether the Agent
    /// answered by the cancel's deadline.
    Cancelled {
        reusable: bool,
        session_invalid: bool,
    },
}

/// Cancels a stalled attempt and, while the schedule allows, re-sends the same prompt.
///
/// The order matters: `session/cancel` first, then the stalled request's response
/// fence, and only then a new `session/prompt`. ACP updates carry no request id, so
/// the fence is the only proof that nothing the abandoned attempt still had in
/// flight can be mistaken for the retry's output. `session/close` is deliberately
/// not part of a retry: it would drop the provider session and force the next send
/// through load or handoff, which is the expensive path the retry exists to avoid.
#[allow(clippy::too_many_arguments)]
pub(super) async fn retry_stalled_prompt<H: AgentRuntimeHost>(
    actor: &mut RuntimeActor<H>,
    channel: &mut SessionChannel,
    pending: ora_acp::PendingSessionRequest<PromptResponse>,
    events: &mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
    liveness: &mut PromptLiveness,
    permissions: &mut HashMap<String, (RequestId, Vec<String>)>,
    request: &mut PromptRequest,
    attempt_instruction: Option<
        &(dyn Fn() -> agent_client_protocol_schema::v1::ContentBlock + Send + Sync),
    >,
    saw_output: &mut bool,
    followers: &mut crate::session_followers::SessionFollowers,
) -> StalledPrompt {
    let snapshot = liveness.snapshot();
    ora_warn!(
        session_id = %actor.session.id,
        policy = ?PromptInactivityPolicy::Timeout,
        attempt = snapshot.attempt,
        window_ms = snapshot.window.as_secs_f64() * 1000.0,
        silent_elapsed_ms = snapshot.silent_elapsed.as_secs_f64() * 1000.0,
        running_tools = snapshot.running_tools,
        pending_tools = snapshot.pending_tools,
        "prompt inactive; cancelling stalled attempt"
    );
    let client = channel.connection.client.clone();
    actor.cancel(&client, permissions).await;
    // The watchdog's own grace applies until a turn cancel merges in; the first merged cancel
    // replaces it with that cancel's deadline once, and the watchdog's cancel is that cancel's.
    let turn_cancel = Arc::clone(&actor.turn_cancel);
    let watchdog_deadline = Instant::now() + CANCELLATION_GRACE;
    let settled = {
        let settle = settle_cancelled_prompt(
            actor, channel, &client, pending, events, saw_output, followers,
        );
        tokio::pin!(settle);
        loop {
            // One snapshot decides both the timer and whether to keep listening, so a cancel that
            // arrives between two reads cannot leave the watchdog's grace in charge.
            let merged = turn_cancel.deadline();
            let deadline = merged.unwrap_or(watchdog_deadline);
            tokio::select! {
                settled = &mut settle => break Ok(settled),
                () = tokio::time::sleep_until(deadline) => break Err(()),
                () = turn_cancel.requested(), if merged.is_none() => {}
            }
        }
    };
    if turn_cancel.deadline().is_some() {
        let reusable = matches!(
            settled,
            Ok(Some(Ok(_))) | Ok(Some(Err(ora_acp::AcpError::RequestFailed(_))))
        );
        if !reusable {
            drain_queued_prompt_events(actor, channel, &client, events, saw_output, followers)
                .await;
        }
        let session_invalid = matches!(&settled, Ok(Some(Err(error))) if super::support::provider_session_invalid(error));
        return StalledPrompt::Cancelled {
            reusable,
            session_invalid,
        };
    }
    let (cancellation_result, stop_reason) = match &settled {
        Ok(Some(Ok(response))) => ("response", Some(response.stop_reason)),
        Ok(Some(Err(_))) => ("request_error", None),
        Ok(None) => ("channel_closed", None),
        Err(_) => ("grace_expired", None),
    };
    ora_warn!(
        session_id = %actor.session.id,
        policy = ?PromptInactivityPolicy::Timeout,
        attempt = snapshot.attempt,
        cancellation_result,
        stop_reason = ?stop_reason,
        "inactive prompt cancellation settled"
    );
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
        let answered = matches!(
            settled,
            Ok(Some(Ok(_))) | Ok(Some(Err(ora_acp::AcpError::RequestFailed(_))))
        );
        if !matches!(settled, Ok(Some(_))) {
            drain_queued_prompt_events(actor, channel, &client, events, saw_output, followers)
                .await;
        }
        // A closed channel while waiting is a lost Agent, not a stalled one (D2/D5), so it is
        // `agent_unavailable`. Otherwise the watchdog gave up on a stalled turn: `no_progress`.
        let failure = if matches!(settled, Ok(None)) {
            crate::turn_failure::agent_unavailable_failure()
        } else {
            no_progress_from_settled(snapshot.attempt, &settled)
        };
        let failure = agent_timed_out("agent prompt made no progress").with_turn_failure(failure);
        return StalledPrompt::Failed(if answered {
            failure.answered()
        } else {
            failure
        });
    };
    ora_warn!(session_id = %actor.session.id, retry = retry.retry, max_retries = retry.max_retries, "prompt inactive; re-sending prompt");
    let outcome = actor.recorder.record_retry();
    let failed = matches!(outcome, super::RecordOutcome::JustFailed { .. });
    actor.settle_record(outcome);
    if failed {
        return StalledPrompt::Failed(super::history_degraded());
    }
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
    if let Some(instruction) = attempt_instruction
        && let Some(block) = request.prompt.last_mut()
    {
        // Admission appended the host-only block last. Rotate it after the response fence;
        // otherwise the abandoned send's pending completion could authorize its replacement.
        *block = instruction();
    }
    // A turn cancel that arrived while the retry was being prepared still wins: the Agent
    // confirmed the stalled attempt cancelled, so nothing is resent and the session stays usable.
    // This check is the last step before the write, as for a first prompt.
    if turn_cancel.deadline().is_some() {
        return StalledPrompt::Cancelled {
            reusable: true,
            session_invalid: false,
        };
    }
    match client
        .start_session_request::<_, PromptResponse>(
            AcpSessionId::new(actor.provider_session_id().to_string()),
            AGENT_METHOD_NAMES.session_prompt,
            request,
        )
        .await
    {
        Ok(pending) => StalledPrompt::Resent(pending),
        // A transport failure on the resend is the Agent's absence, as on the first send (D2/D7).
        Err(error) => StalledPrompt::Failed(map_prompt_error(error)),
    }
}

/// Builds the `no_progress` failure the watchdog records when it gives up: how many
/// `session/prompt` writes the turn made, and how the Agent answered the last cancel.
pub(super) fn no_progress_from_settled<E>(
    attempts: usize,
    settled: &Result<Option<Result<PromptResponse, ora_acp::AcpError>>, E>,
) -> TurnFailure {
    let attempts = u32::try_from(attempts).unwrap_or(u32::MAX);
    let (answer, acp_code, message, message_truncated) = match settled {
        Ok(Some(Ok(response))) => {
            let answer = match response.stop_reason {
                StopReason::Cancelled => TurnFailureAnswer::Cancelled,
                StopReason::EndTurn => TurnFailureAnswer::EndTurn,
                StopReason::MaxTokens => TurnFailureAnswer::MaxTokens,
                StopReason::MaxTurnRequests => TurnFailureAnswer::MaxTurnRequests,
                StopReason::Refusal => TurnFailureAnswer::Refusal,
                _ => TurnFailureAnswer::None,
            };
            (answer, None, None, None)
        }
        Ok(Some(Err(ora_acp::AcpError::RequestFailed(request)))) => {
            let sanitized =
                crate::turn_failure::sanitize_agent_text(&request.message, &request.data);
            (
                TurnFailureAnswer::Error,
                Some(request.code),
                sanitized.as_ref().map(|(text, _)| text.clone()),
                sanitized
                    .as_ref()
                    .and_then(|(_, truncated)| (*truncated).then_some(true)),
            )
        }
        Ok(Some(Err(_))) => (TurnFailureAnswer::Error, None, None, None),
        Ok(None) | Err(_) => (TurnFailureAnswer::None, None, None, None),
    };
    crate::turn_failure::no_progress_failure(attempts, answer, acp_code, message, message_truncated)
}
