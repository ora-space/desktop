//! Retains operation-specific evidence; a later turn must not validate an older failed cancel.
use super::*;
use crate::AgentRuntimeHost;

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Bounds retained evidence; an evicted receipt becomes unconfirmed rather than successful.
    pub(crate) fn record_cleanup(&mut self, operation_id: u64, result: Result<(), RuntimeError>) {
        if self.cleanup_outcomes.len() >= 256
            && !self.cleanup_outcomes.contains_key(&operation_id)
            && let Some(oldest) = self.cleanup_outcomes.keys().min().copied()
        {
            self.cleanup_outcomes.remove(&oldest);
        }
        self.cleanup_outcomes.insert(operation_id, result);
    }

    /// Reuses only this operation's evidence, regardless of the actor's current provider binding.
    pub(super) fn cleanup_outcome(&self, operation_id: u64) -> Result<(), RuntimeError> {
        self.cleanup_outcomes
            .get(&operation_id)
            .cloned()
            .unwrap_or_else(|| {
                Err(RuntimeError::internal(
                    "operation cleanup cannot be confirmed",
                    std::io::Error::other("missing or expired operation receipt"),
                ))
            })
    }
    /// Cancels the provider turn and settles every outstanding permission request.
    pub(crate) async fn cancel(
        &self,
        client: &AgentAcpClient,
        permissions: &HashMap<String, (agent_client_protocol_schema::v1::RequestId, Vec<String>)>,
    ) {
        ora_debug!(session_id = %self.session.id, pending_permissions = permissions.len(), "cancelling prompt");
        crate::prompt_permissions::cancel_pending(client, permissions).await;
        let _ = client
            .notify(
                AGENT_METHOD_NAMES.session_cancel,
                &CancelNotification::new(self.provider_session_id().to_string()),
            )
            .await;
    }

    /// Closes only this live ACP registration and preserves provider-owned history.
    pub(super) async fn unload(&mut self) {
        if let Some(channel) = self.channel.take() {
            self.close_provider_session(&channel).await;
            self.persist_session_status(SessionStatus::Stopped);
        } else {
            self.persist_session_status(SessionStatus::Stopped);
        }
    }

    /// Detaches from the provider without recording any lifecycle change.
    ///
    /// Used only when the manager retires this actor, because it owns the row's
    /// next state and this actor's view of it is already out of date.
    pub(super) async fn release(&mut self) {
        self.title_acquisition.close();
        if let Some(channel) = self.channel.take() {
            self.close_provider_session(&channel).await;
        }
    }

    /// Detaches one routed session while leaving the shared CLI process available.
    pub(crate) async fn isolate_channel(&mut self, channel: SessionChannel) {
        self.title_acquisition.close();
        self.close_provider_session(&channel).await;
        self.mark_stopped();
    }

    /// Releases the provider-side registration when the agent advertises the call.
    async fn close_provider_session(&self, channel: &SessionChannel) {
        if channel.connection.close_session_supported {
            let _ = timeout(
                CANCELLATION_GRACE,
                channel
                    .connection
                    .client
                    .request::<_, CloseSessionResponse>(
                        AGENT_METHOD_NAMES.session_close,
                        &CloseSessionRequest::new(self.provider_session_id().to_string()),
                    ),
            )
            .await;
        }
    }
}
