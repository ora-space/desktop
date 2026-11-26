//! Ends a prompt the provider failed to answer.
//!
//! Kept apart from the prompt loop so the failure paths — an Agent request error, a transport
//! failure, and the single retry of a stale restored session — stay readable together.

use super::*;
use crate::actor::publish_setup;
use crate::support::{map_prompt_error, provider_session_invalid};

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Handles a prompt the provider failed to answer: distinguishes the Agent's own request error
    /// from a transport failure, retries a stale restored session once, and otherwise ends the turn
    /// with the classified failure recorded (D2/D3).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn handle_prompt_failure(
        &mut self,
        operation_id: u64,
        original_prompt: Vec<ContentBlock>,
        record_prompt: RecordedTurn,
        inactivity_policy: PromptInactivityPolicy,
        channel: SessionChannel,
        error: ora_acp::AcpError,
        events: mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
        end_report: PromptEndReport,
        mut followers: SessionFollowers,
        tool_timings: &ToolTimings,
        saw_output: bool,
        retry_cutoff: Option<u64>,
    ) {
        let reusable = matches!(&error, ora_acp::AcpError::RequestFailed(_));
        let session_invalid = provider_session_invalid(&error);
        // A load can claim success while the provider session is gone; its first prompt then fails
        // before producing anything. Rebuild once and resend the same prompt with the transcript
        // this side still owns.
        let retry_blocks =
            retry_cutoff.and_then(|cutoff| rebuild_prompt(self, &original_prompt, cutoff));
        let retry_stale_load = stale_load_retry_allowed(
            reusable,
            saw_output,
            retry_blocks.is_some(),
            self.load_restoration,
        );
        if retry_stale_load && !self.session_mcp.preserve_provider_identity() {
            ora_debug!(
                session_id = %self.session.id,
                "first prompt after session/load failed without output; rebuilding and retrying",
            );
            match self.replace_stale_session(channel).await {
                Ok(setup) => {
                    for update in &setup {
                        followers.send_update(update, None);
                    }
                    if !publish_setup(&events, setup) {
                        self.record_cleanup(operation_id, Ok(()));
                        end_report.record(PromptEndCause::StreamDropped);
                        self.end_timed_turn(StopReason::Cancelled, tool_timings);
                        followers.finish(StopReason::Cancelled);
                        return;
                    }
                    return Box::pin(self.run_prompt_attempt(
                        operation_id,
                        original_prompt,
                        record_prompt,
                        inactivity_policy,
                        end_report,
                        events,
                        PromptAttempt {
                            blocks: retry_blocks,
                            skip_record: true,
                            allow_retry: false,
                            followers: Some(followers),
                        },
                    ))
                    .await;
                }
                Err(rebuild_error) => {
                    ora_warn!(
                        session_id = %self.session.id,
                        error = %rebuild_error,
                        "failed to rebuild a stale restored session; returning the prompt failure",
                    );
                    self.record_cleanup(operation_id, Ok(()));
                    end_report.record(PromptEndCause::Unknown);
                    self.end_timed_turn(StopReason::Cancelled, tool_timings);
                    followers.finish(StopReason::Cancelled);
                    self.load_restoration = LoadRestoration::NotRestored;
                    let _ = events.try_send(Err(map_prompt_error(error)));
                    return;
                }
            }
        }
        if reusable {
            self.record_cleanup(operation_id, Ok(()));
        }
        ora_debug!(session_id = %self.session.id, code = "agent_turn_failed", reusable = reusable, "prompt failed");
        end_report.record(PromptEndCause::Unknown);
        let failure = map_prompt_error(error);
        // A request error is the Agent's own answer; a transport failure is not.
        let failure = if reusable {
            failure.answered()
        } else {
            failure
        };
        let turn_failure = failure.turn_failure().cloned();
        self.end_timed_turn_with(StopReason::Cancelled, tool_timings, None, turn_failure);
        followers.finish(StopReason::Cancelled);
        self.load_restoration = LoadRestoration::NotRestored;
        let delivered = events.try_send(Err(failure)).is_ok();
        self.end_on_channel(channel, reusable && !session_invalid, delivered)
            .await;
    }
}
