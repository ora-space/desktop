//! Replaces a restored session whose first prompt proved the provider no longer holds it.

use super::*;
use crate::model_config::current_model;
use agent_client_protocol_schema::v1::AvailableCommandsUpdate;

/// The one retry after a load is allowed only before output and only once.
pub(super) fn stale_load_retry_allowed(
    reusable: bool,
    produced_output: bool,
    has_context: bool,
    restoration: LoadRestoration,
) -> bool {
    reusable && !produced_output && has_context && restoration == LoadRestoration::Restored
}

/// Whether an update describes the provider session rather than the turn running on it.
///
/// A follower keeps showing the session it joined on until it is told otherwise, so these have to
/// reach it when that session is replaced mid-turn.
pub(super) fn describes_provider(update: &SessionUpdate) -> bool {
    matches!(
        update,
        SessionUpdate::AvailableCommandsUpdate(_) | SessionUpdate::ConfigOptionUpdate(_)
    )
}

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Releases the stale session, opens its replacement and returns what that one reported.
    ///
    /// Commands the stale session had reported are withdrawn when the replacement brings none during
    /// setup: what is still on screen belongs to a session that is gone. Its options always lead
    /// the setup, so those are replaced either way.
    pub(super) async fn replace_stale_session(
        &mut self,
        mut channel: SessionChannel,
    ) -> Result<Vec<SessionUpdate>, RuntimeError> {
        let model = current_model(&self.reported_config_options);
        let had_commands = channel.reported_commands().is_some();
        // Their answers can no longer arrive. They are made as far as the caller is concerned, and
        // leave it on what the replacement offers.
        let mut changes = std::mem::take(&mut channel.config_changes);
        self.isolate_channel(channel).await;
        let mut setup = self.rebuild_provider_session(model.as_deref()).await?;
        changes.accept();
        // Commands usually arrive only after setup, and then replace this empty catalog.
        let reported = setup
            .iter()
            .any(|update| matches!(update, SessionUpdate::AvailableCommandsUpdate(_)));
        if had_commands && !reported {
            setup.push(SessionUpdate::AvailableCommandsUpdate(
                AvailableCommandsUpdate::new(Vec::new()),
            ));
        }
        Ok(setup)
    }
}
