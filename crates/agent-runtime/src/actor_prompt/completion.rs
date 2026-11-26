//! Ends a prompt the provider answered.

use super::*;

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Ends a prompt the provider answered with a result: its stop reason describes the turn, its
    /// usage is reported, and a completed turn may start title acquisition. Shared by the prompt
    /// loop and by a turn cancel that found the answer already queued.
    pub(crate) async fn finish_completed_prompt(
        &mut self,
        mut channel: SessionChannel,
        response: PromptResponse,
        operation_id: u64,
        end: crate::turn_cancel::PromptEnd<'_>,
    ) {
        let crate::turn_cancel::PromptEnd {
            events,
            end_report,
            followers,
            tool_timings,
        } = end;
        self.record_cleanup(operation_id, Ok(()));
        // The provider answered: its stop reason now describes the turn, so the end cause is not
        // an interruption.
        end_report.record(PromptEndCause::Settled);
        ora_debug!(session_id = %self.session.id, stop_reason = ?response.stop_reason, "prompt completed");
        let token_usage = usage::normalize_token_usage(
            &self.session.agent_ref,
            response.usage.as_ref(),
            response.meta.as_ref(),
            &usage::NoUsageExtensions,
        );
        self.end_timed_turn(response.stop_reason, tool_timings);
        followers.finish(response.stop_reason);
        self.load_restoration = LoadRestoration::NotRestored;
        self.maybe_start_title_acquisition(response.stop_reason);
        if events
            .try_send(Ok(PromptSessionEvent::Completed {
                stop_reason: response.stop_reason,
                token_usage,
            }))
            .is_ok()
        {
            self.channel = Some(channel);
        } else {
            self.accept_changes_on(&mut channel);
            self.isolate_channel(channel).await;
        }
    }
}
