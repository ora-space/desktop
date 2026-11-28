use super::*;
use crate::AgentRuntimeHost;
use ora_history::{TurnEndedBy, TurnFailure};

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Closes the recorded turn and records why a turn that did not complete ended.
    pub(crate) fn end_turn_with(
        &mut self,
        stop_reason: StopReason,
        ended_by: Option<TurnEndedBy>,
        failure: Option<TurnFailure>,
    ) {
        let outcome = self
            .recorder
            .record_turn_end_with(stop_reason, ended_by, failure);
        self.settle_record(outcome);
    }

    /// Freezes this prompt's tool durations before history settles its open snapshots.
    pub(crate) fn end_timed_turn(&mut self, stop_reason: StopReason, timings: &ToolTimings) {
        self.end_timed_turn_with(stop_reason, timings, None, None);
    }

    /// Freezes this prompt's tool durations and records why a turn that did not complete ended.
    pub(crate) fn end_timed_turn_with(
        &mut self,
        stop_reason: StopReason,
        timings: &ToolTimings,
        ended_by: Option<TurnEndedBy>,
        failure: Option<TurnFailure>,
    ) {
        self.recorder.finish_tool_timings(timings.finish_all());
        self.end_turn_with(stop_reason, ended_by, failure);
        // Any terminal prompt consumes the restored channel's one automatic-retry eligibility,
        // including a user cancellation the agent confirmed.
        self.load_restoration = Default::default();
    }

    /// Streams Ora's recorded conversation to a client that loaded it.
    ///
    /// Sends apply backpressure rather than failing fast: a long history is far
    /// larger than the event queue, and a slow consumer is not a disconnected one.
    pub(super) async fn replay_recorded_history(
        &self,
        events: &mpsc::Sender<Result<LoadSessionEvent, RuntimeError>>,
    ) -> Replay {
        let history = match read_session_history(&self.sessions_root, self.session.id.as_ref()) {
            Ok(history) => history,
            Err(error) => {
                // Load is how a user asks to see the conversation, so a history
                // that cannot be read is reported rather than shown as an empty
                // one. Completing here would state that nothing was ever said.
                ora_warn!(session_id = %self.session.id, error = %error, "session history unreadable during load");
                let _ = events.send(Err(session_history_unreadable())).await;
                return Replay::Unreadable;
            }
        };
        for event in recorded_replay(history) {
            if events.send(Ok(event)).await.is_err() {
                return Replay::Abandoned;
            }
        }
        Replay::Delivered
    }
}
