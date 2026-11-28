//! Settlement of a workflow prompt whose finite silence window elapsed.
use super::events::{drain_queued_prompt_events, settle_cancelled_prompt};
use super::limits::CANCELLATION_GRACE;
use super::routing::SessionChannel;
use super::{RuntimeActor, agent_timed_out};
use crate::AgentRuntimeHost;
use agent_client_protocol_schema::v1::{PromptResponse, StopReason};
use ora_domain::PromptInactivityPolicy;
use ora_logging::ora_warn;

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Settle the old provider request before handing its single-send failure to the node.
    pub(super) async fn finish_expired_wait(
        &mut self,
        mut channel: SessionChannel,
        pending: ora_acp::PendingSessionRequest<PromptResponse>,
        snapshot: super::prompt_liveness::PromptLivenessSnapshot,
        stop: crate::turn_cancel::PromptStop<'_>,
    ) {
        let crate::turn_cancel::PromptStop {
            operation_id,
            client,
            permissions,
            events,
            end_report,
            followers,
            tool_timings,
            saw_output,
        } = stop;
        end_report.record(crate::PromptEndCause::InactivityTimeout);
        let cleanup_deadline = tokio::time::Instant::now() + CANCELLATION_GRACE;
        let settled = tokio::time::timeout_at(cleanup_deadline, async {
            self.cancel(client, permissions).await;
            settle_cancelled_prompt(
                self,
                &mut channel,
                client,
                pending,
                events,
                saw_output,
                followers,
            )
            .await
        })
        .await;
        let cancellation_result = match &settled {
            Ok(Some(Ok(response))) if response.stop_reason == StopReason::Cancelled => "confirmed",
            Ok(Some(Ok(_))) => "other_response",
            Ok(Some(Err(_))) => "request_error",
            Ok(None) => "channel_closed",
            Err(_) => "grace_expired",
        };
        ora_warn!(session_id = %self.session.id, operation_id, policy = ?PromptInactivityPolicy::Wait,
                        attempt = snapshot.attempt, window_ms = snapshot.window.as_secs_f64() * 1000.0,
                        silent_elapsed_ms = snapshot.silent_elapsed.as_secs_f64() * 1000.0,
                        running_tools = snapshot.running_tools, pending_tools = snapshot.pending_tools,
                        end_cause = "inactivity_timeout", cancellation_result,
                        "workflow prompt exceeded its silence limit");
        let failure = if matches!(settled, Ok(None)) {
            crate::turn_failure::agent_unavailable_failure()
        } else {
            super::prompt_retry::no_progress_from_settled(snapshot.attempt, &settled)
        };
        let mut error = agent_timed_out(format!(
            "workflow agent reported no progress for {} minutes",
            snapshot.window.as_secs() / 60
        ))
        .with_turn_failure(failure);
        if cancellation_result == "confirmed" {
            self.record_cleanup(operation_id, Ok(()));
            self.channel = Some(channel);
            error = error.answered();
        } else {
            let _ = tokio::time::timeout_at(
                cleanup_deadline,
                drain_queued_prompt_events(
                    self,
                    &mut channel,
                    client,
                    events,
                    saw_output,
                    followers,
                ),
            )
            .await;
            self.isolate_channel(channel).await;
        }
        let turn_failure = error.turn_failure().cloned();
        self.end_timed_turn_with(StopReason::Cancelled, tool_timings, None, turn_failure);
        followers.finish_with_result(StopReason::Cancelled, Err(error.clone()));
        let _ = events.try_send(Err(error));
    }
}
