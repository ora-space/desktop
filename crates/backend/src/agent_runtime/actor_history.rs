use super::*;

impl RuntimeActor {
    /// Closes the recorded turn after the ordered event consumer has settled its events.
    pub(super) fn end_turn(&mut self, stop_reason: StopReason) {
        let outcome = self.recorder.record_turn_end(stop_reason);
        self.settle_record(outcome);
    }

    /// Freezes this prompt's tool durations before history settles its open snapshots.
    pub(super) fn end_timed_turn(&mut self, stop_reason: StopReason, timings: &ToolTimings) {
        self.recorder.finish_tool_timings(timings.finish_all());
        self.end_turn(stop_reason);
    }

    /// Streams Ora's recorded conversation to a client that loaded it.
    ///
    /// Sends apply backpressure rather than failing fast: a long history is far
    /// larger than the event queue, and a slow consumer is not a disconnected one.
    pub(super) async fn replay_recorded_history(
        &self,
        events: &mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
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
