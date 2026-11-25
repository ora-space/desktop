//! Admits a load of the session whose prompt is running.

use super::*;

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Extends the running prompt to one more view of its session.
    ///
    /// The durable cutoff, the in-progress pending records, and the live follower registration are
    /// captured atomically (no await); the follower's relay task then streams the merged prefix
    /// before live events. The actor returns to its select loop immediately, so a slow view can
    /// never backpressure the prompt.
    pub(super) fn follow_prompt(
        &mut self,
        operation_id: u64,
        channel: &SessionChannel,
        followers: &mut SessionFollowers,
        output: FollowerOutput,
        accepted: oneshot::Sender<Result<(), RuntimeError>>,
    ) {
        self.record_cleanup(operation_id, Ok(()));
        let cutoff = self.recorder.durable_bytes();
        let pending = self.recorder.pending_records();
        if accepted.send(Ok(())).is_err() {
            return;
        }
        // The follower's queue is new and empty, so these precede its replay. Without the options a
        // view would take the session for one that holds no provider to configure.
        if !self.reported_config_options.is_empty() {
            let update = SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
                self.reported_config_options.clone(),
            ));
            let _ = output
                .events
                .try_send(Ok(LoadSessionEvent::session_update(update)));
        }
        if let Some(update) = channel.reported_commands() {
            let _ = output
                .events
                .try_send(Ok(LoadSessionEvent::session_update(update)));
        }
        followers.insert(
            operation_id,
            output,
            self.sessions_root.clone(),
            self.session.id.to_string(),
            cutoff,
            pending,
        );
    }
}
