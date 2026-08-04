use super::routing::SessionControl;
use super::*;
use ora_acp::AcpClient;
use ora_contracts::SessionPermissionRequest;
use ora_contracts::acp::common::SessionId as AcpSessionId;
use ora_contracts::acp::content::TextContent;
use ora_contracts::acp::literals::AGENT_METHOD_NAMES;
use ora_contracts::acp::notification::CancelNotification;
use ora_contracts::acp::permission::{RequestPermissionOutcome, RequestPermissionResponse};
use ora_contracts::acp::prompt::{PromptRequest, PromptResponse, StopReason};
use ora_contracts::acp::session::{
    CloseSessionRequest, CloseSessionResponse, LoadSessionRequest as AcpLoadSessionRequest,
    LoadSessionResponse,
};
use ora_history::{HistoryRecord, render_handoff};
use ora_logging::{ora_debug, ora_warn};
use tokio::process::ChildStdin;
use tokio::time::{Instant, timeout};

/// How far replaying Ora's record got before it stopped.
///
/// Only `Delivered` may complete the load. The other two are kept apart because
/// they differ in who still has to be told: an unreadable history owes the
/// client an error, while an abandoned one has no client left to send it to.
enum Replay {
    /// Every recorded line reached the client.
    Delivered,
    /// The history could not be read, and the client was told so.
    Unreadable,
    /// The client stopped listening partway through.
    Abandoned,
}

impl RuntimeActor {
    /// Serializes operations for one logical session while the shared connection remains concurrent.
    pub(super) async fn run(mut self) {
        loop {
            let command = match self.channel.as_mut() {
                Some(channel) => {
                    tokio::select! {
                        biased;
                        command = self.commands.recv() => command,
                        control = channel.controls.recv() => {
                            self.handle_idle_control(control).await;
                            continue;
                        }
                        update = channel.updates.recv() => {
                            if update.is_none() {
                                self.mark_stopped();
                            }
                            continue;
                        }
                    }
                }
                None => self.commands.recv().await,
            };
            let Some(command) = command else {
                // The manager dropped this actor, which it only does when it is
                // replacing or deleting the row itself. Detaching from the
                // provider is still required, but persisting anything would write
                // a snapshot the manager has already moved past — that is exactly
                // how a switch's new binding gets reverted to Stopped.
                self.release().await;
                return;
            };
            match command {
                RuntimeCommand::Load {
                    operation_id,
                    events,
                    accepted,
                } => {
                    let _ = accepted.send(Ok(()));
                    self.run_load(operation_id, events).await;
                }
                RuntimeCommand::Prompt {
                    operation_id,
                    prompt,
                    events,
                    accepted,
                } => {
                    if self.channel.is_none() {
                        let _ = accepted.send(Err(session_stopped()));
                    } else {
                        let _ = accepted.send(Ok(()));
                        self.run_prompt(operation_id, prompt, events).await;
                    }
                }
                RuntimeCommand::RespondToPermission { response, .. } => {
                    let _ = response.send(Err(permission_not_pending()));
                }
                RuntimeCommand::Stop { response } => {
                    self.unload().await;
                    let _ = response.send(Ok(StopSessionResponse {
                        session: contract_session(self.session.clone()),
                    }));
                }
                RuntimeCommand::Cancel { .. } => {}
            }
        }
    }

    /// Re-registers a stopped session and streams provider history without replacing the process.
    async fn run_load(
        &mut self,
        operation_id: u64,
        events: mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
    ) {
        self.unload().await;
        let running = self
            .session
            .clone()
            .with_status(SessionStatus::Running, self.clock.now_timestamp_millis());
        if self.repository.update_session(running.clone()).is_err() {
            let _ = events.try_send(Err(session_not_found(self.session.id.as_ref())));
            return;
        }
        self.session = running;
        let channel = match self
            .connection
            .open_session_channel(&self.session.agent_session_id)
        {
            Ok(channel) => channel,
            Err(error) => {
                let _ = events.try_send(Err(error));
                self.mark_stopped();
                return;
            }
        };
        if !channel.connection.load_session_supported {
            let _ = events.try_send(Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::SessionLoadUnsupported(EmptyErrorParams {}),
                "agent CLI does not support session/load",
            )));
            self.mark_stopped();
            return;
        }
        self.run_load_on_channel(operation_id, events, channel)
            .await;
    }

    /// Selects over the provider handshake, routed updates, cancellation, and connection failure.
    ///
    /// `session/load` is still called, but only so the agent restores the context
    /// it needs to answer the next prompt. Its replay is drained and discarded:
    /// what the client is shown comes from Ora's own record, which is the same
    /// conversation whichever agent is currently bound to it.
    async fn run_load_on_channel(
        &mut self,
        operation_id: u64,
        events: mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
        mut channel: SessionChannel,
    ) {
        let client = channel.connection.client.clone();
        let request = AcpLoadSessionRequest::new(
            AcpSessionId::new(self.session.agent_session_id.clone()),
            &self.cwd,
        );
        ora_debug!(session_id = %self.session.id, "session/load sent");
        let future =
            client.request::<_, LoadSessionResponse>(AGENT_METHOD_NAMES.session_load, &request);
        tokio::pin!(future);
        let deadline = tokio::time::sleep(SESSION_SETUP_TIMEOUT);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                response = &mut future => {
                    match response {
                        Ok(_) => {
                            ora_debug!(session_id = %self.session.id, "session/load completed");
                            match self.replay_recorded_history(&events).await {
                                Replay::Delivered
                                    if events.send(Ok(LoadSessionEvent::Completed)).await.is_ok() =>
                                {
                                    self.channel = Some(channel);
                                }
                                // A replay that did not finish leaves the client
                                // without the conversation it asked for, so the
                                // registration goes with it and load can be retried.
                                Replay::Delivered | Replay::Unreadable | Replay::Abandoned => {
                                    self.isolate_channel(channel).await;
                                }
                            }
                        }
                        Err(error) => {
                            ora_debug!(session_id = %self.session.id, error = %error, "session/load failed");
                            let _ = events.try_send(Err(map_acp_error(error)));
                            self.isolate_channel(channel).await;
                        }
                    }
                    return;
                }
                update = channel.updates.recv() => {
                    if update.is_none() {
                        self.fail_load(&events, runtime_unavailable());
                        return;
                    }
                    // The agent is reciting history Ora already owns. Draining it
                    // keeps the queue clear and proves the agent is still working.
                    deadline.as_mut().reset(Instant::now() + SESSION_SETUP_TIMEOUT);
                }
                control = channel.controls.recv() => {
                    match control {
                        Some(SessionControl::Permission(permission)) => {
                            let _ = client.respond(
                                &permission.request_id,
                                &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                            ).await;
                            let _ = events.try_send(Err(runtime_internal(
                                "agent_protocol_error",
                                "permission request during session/load is unsupported",
                            )));
                            self.isolate_channel(channel).await;
                            return;
                        }
                        Some(SessionControl::ConnectionLost(error)) => {
                            self.fail_load(&events, error);
                        }
                        Some(SessionControl::UpdateOverflow) => {
                            let _ = events.try_send(Err(runtime_internal(
                                "agent_update_overflow",
                                "session update queue overflowed",
                            )));
                            self.isolate_channel(channel).await;
                            return;
                        }
                        None => self.fail_load(&events, runtime_unavailable()),
                    }
                    return;
                }
                _ = &mut deadline => {
                    ora_debug!(session_id = %self.session.id, "session/load timed out");
                    self.cancel(&client, &HashMap::new()).await;
                    let _ = events.try_send(Err(runtime_internal(
                        "agent_load_timeout",
                        "agent CLI session load timed out",
                    )));
                    self.isolate_channel(channel).await;
                    return;
                }
                command = self.commands.recv() => {
                    match command {
                        Some(RuntimeCommand::Cancel { operation_id: cancelled })
                            if cancelled == operation_id =>
                        {
                            self.cancel(&client, &HashMap::new()).await;
                            self.isolate_channel(channel).await;
                            return;
                        }
                        Some(RuntimeCommand::Stop { response }) => {
                            self.cancel(&client, &HashMap::new()).await;
                            self.isolate_channel(channel).await;
                            let _ = response.send(Ok(StopSessionResponse {
                                session: contract_session(self.session.clone()),
                            }));
                            return;
                        }
                        Some(RuntimeCommand::Prompt { accepted, .. })
                        | Some(RuntimeCommand::Load { accepted, .. }) => {
                            let _ = accepted.send(Err(session_busy()));
                        }
                        Some(RuntimeCommand::RespondToPermission { response, .. }) => {
                            let _ = response.send(Err(permission_not_pending()));
                        }
                        Some(RuntimeCommand::Cancel { .. }) | None => {}
                    }
                }
            }
        }
    }

    /// Streams one prompt while routing only events that belong to this provider session.
    async fn run_prompt(
        &mut self,
        operation_id: u64,
        prompt: Vec<ContentBlock>,
        events: mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
    ) {
        let Some(mut channel) = self.channel.take() else {
            return;
        };
        while let Some(notification) = channel.pending_updates.pop_front() {
            if events
                .try_send(Ok(PromptSessionEvent::SessionUpdate {
                    update: notification.update,
                }))
                .is_err()
            {
                self.isolate_channel(channel).await;
                return;
            }
        }
        let client = channel.connection.client.clone();
        let content_count = prompt.len();
        // Built before the prompt is recorded, so the transcript handed to a new
        // agent describes the conversation up to this turn rather than including it.
        let handoff_carried = self.handoff_pending;
        let sent = self.prompt_for_agent(&prompt);
        let outcome = self.recorder.record_prompt(&prompt);
        let stopped_recording = matches!(outcome, RecordOutcome::JustFailed { .. });
        self.settle_record(outcome);
        if stopped_recording {
            // A turn already streaming is allowed to finish, because the agent's
            // work is real whether or not the file kept it. This one has not
            // started: nothing is lost by refusing it, and running it would put
            // the conversation somewhere the record cannot follow.
            //
            // The transcript this prompt would have carried was never delivered
            // either, so the binding still owes it.
            self.handoff_pending = handoff_carried;
            let _ = events.try_send(Err(history_degraded()));
            self.channel = Some(channel);
            return;
        }
        let request = PromptRequest::new(self.session.agent_session_id.clone(), sent);
        ora_debug!(session_id = %self.session.id, content_count = content_count, "session/prompt sent");
        let future =
            client.request::<_, PromptResponse>(AGENT_METHOD_NAMES.session_prompt, &request);
        tokio::pin!(future);
        let mut permissions = HashMap::new();
        loop {
            tokio::select! {
                response = &mut future => {
                    match response {
                        Ok(response) => {
                            ora_debug!(session_id = %self.session.id, stop_reason = ?response.stop_reason, "prompt completed");
                            self.end_turn(&mut channel, &events, response.stop_reason);
                            if events.try_send(Ok(PromptSessionEvent::Completed {
                                stop_reason: response.stop_reason,
                            })).is_ok() {
                                self.channel = Some(channel);
                            } else {
                                self.isolate_channel(channel).await;
                            }
                        }
                        Err(error) => {
                            let reusable = matches!(&error, ora_acp::AcpError::RequestFailed(_));
                            ora_debug!(session_id = %self.session.id, error = %error, reusable = reusable, "prompt failed");
                            // A turn that failed did not reach its own end, which
                            // is the same shape in the record as one cut short.
                            self.end_turn(&mut channel, &events, StopReason::Cancelled);
                            let delivered = events.try_send(Err(map_acp_error(error))).is_ok();
                            if reusable && delivered {
                                self.channel = Some(channel);
                            } else {
                                self.isolate_channel(channel).await;
                            }
                        }
                    }
                    return;
                }
                update = channel.updates.recv() => {
                    let Some(update) = update else {
                        self.end_turn(&mut channel, &events, StopReason::Cancelled);
                        self.fail_prompt(&events, runtime_unavailable());
                        return;
                    };
                    // Recorded before it is forwarded: a client that drops mid-turn
                    // must not also cost the record of what the agent produced.
                    let outcome = self.recorder.record_update(&update.update);
                    self.settle_record(outcome);
                    if events.try_send(Ok(PromptSessionEvent::SessionUpdate { update: update.update })).is_err() {
                        self.end_turn(&mut channel, &events, StopReason::Cancelled);
                        self.cancel(&client, &permissions).await;
                        self.isolate_channel(channel).await;
                        return;
                    }
                }
                control = channel.controls.recv() => {
                    match control {
                        Some(SessionControl::Permission(permission)) => {
                            let public_id = permission.request_id.to_string();
                            let option_ids = permission.request.options.iter()
                                .map(|option| option.option_id.to_string())
                                .collect::<Vec<_>>();
                            ora_debug!(session_id = %self.session.id, tool_call = ?permission.request.tool_call, option_count = option_ids.len(), request_id = %public_id, "permission requested");
                            permissions.insert(public_id.clone(), (permission.request_id, option_ids));
                            let event = PromptSessionEvent::PermissionRequest(SessionPermissionRequest {
                                permission_request_id: public_id,
                                tool_call: permission.request.tool_call,
                                options: permission.request.options,
                            });
                            if events.try_send(Ok(event)).is_err() {
                                self.end_turn(&mut channel, &events, StopReason::Cancelled);
                                self.cancel(&client, &permissions).await;
                                self.isolate_channel(channel).await;
                                return;
                            }
                        }
                        Some(SessionControl::ConnectionLost(error)) => {
                            self.end_turn(&mut channel, &events, StopReason::Cancelled);
                            self.fail_prompt(&events, error);
                            return;
                        }
                        Some(SessionControl::UpdateOverflow) => {
                            self.end_turn(&mut channel, &events, StopReason::Cancelled);
                            self.cancel(&client, &permissions).await;
                            let _ = events.try_send(Err(runtime_internal(
                                "agent_update_overflow",
                                "session update queue overflowed",
                            )));
                            self.isolate_channel(channel).await;
                            return;
                        }
                        None => {
                            self.end_turn(&mut channel, &events, StopReason::Cancelled);
                            self.fail_prompt(&events, runtime_unavailable());
                            return;
                        }
                    }
                }
                command = self.commands.recv() => {
                    match command {
                        Some(RuntimeCommand::RespondToPermission { request, response }) => {
                            let result = respond_permission(&client, request, &mut permissions).await;
                            let _ = response.send(result);
                        }
                        Some(RuntimeCommand::Cancel { operation_id: cancelled }) if cancelled == operation_id => {
                            self.cancel(&client, &permissions).await;
                            let settled = timeout(CANCELLATION_GRACE, &mut future).await;
                            // Closing the turn also collects whatever the agent
                            // emitted on its way out, which the grace period above
                            // was not watching for.
                            self.end_turn(&mut channel, &events, StopReason::Cancelled);
                            match settled {
                                Ok(Ok(_)) | Ok(Err(ora_acp::AcpError::RequestFailed(_))) => {
                                    self.channel = Some(channel);
                                }
                                Ok(Err(_)) | Err(_) => self.isolate_channel(channel).await,
                            }
                            return;
                        }
                        Some(RuntimeCommand::Stop { response }) => {
                            self.cancel(&client, &permissions).await;
                            self.end_turn(&mut channel, &events, StopReason::Cancelled);
                            self.isolate_channel(channel).await;
                            let _ = response.send(Ok(StopSessionResponse {
                                session: contract_session(self.session.clone()),
                            }));
                            return;
                        }
                        Some(RuntimeCommand::Prompt { accepted, .. })
                        | Some(RuntimeCommand::Load { accepted, .. }) => {
                            let _ = accepted.send(Err(session_busy()));
                        }
                        Some(RuntimeCommand::Cancel { .. }) | None => {}
                    }
                }
            }
        }
    }

    /// Builds what actually reaches the agent, which is not always what is recorded.
    ///
    /// A binding that has never been told anything gets the recorded transcript
    /// ahead of the user's words. That block is deliberately not recorded: it is
    /// derived from the history, and storing it would nest the conversation inside
    /// itself on the next switch.
    fn prompt_for_agent(&mut self, prompt: &[ContentBlock]) -> Vec<ContentBlock> {
        if !self.handoff_pending {
            return prompt.to_vec();
        }
        let history = match read_session_history(&self.sessions_root, self.session.id.as_ref()) {
            Ok(history) => history,
            Err(error) => {
                // The flag is held rather than cleared, because nothing would ask
                // again. It is derived from the record — a trailing `AgentSwitched`
                // with no user message after it — and recording this prompt is
                // exactly what stops that from being true. A transient read would
                // otherwise cost the new agent the whole conversation, silently and
                // for good, while the turn it answered without any of it becomes
                // history the agent after that one inherits as fact.
                ora_warn!(
                    session_id = %self.session.id,
                    error = %error,
                    "handoff transcript unreadable; retrying on the next prompt",
                );
                return prompt.to_vec();
            }
        };
        self.handoff_pending = false;
        // Nothing recorded to hand over, which is a session switched before it was
        // ever prompted. The binding has now been spoken to either way.
        let Some(transcript) = render_handoff(&history) else {
            return prompt.to_vec();
        };
        ora_debug!(
            session_id = %self.session.id,
            transcript_bytes = transcript.len(),
            "prepending recorded transcript for a new agent binding",
        );
        let mut sent = Vec::with_capacity(prompt.len() + 1);
        sent.push(ContentBlock::Text(TextContent::new(transcript)));
        sent.extend_from_slice(prompt);
        sent
    }

    /// Closes the recorded turn once everything it produced is in hand.
    ///
    /// The select loop stops consuming updates the moment another branch wins —
    /// a resolved response, a cancellation, a lost connection — and during the
    /// cancellation grace it is not running at all. The agent's last updates are
    /// usually already queued behind that, so they are drained here and settled
    /// into the turn that produced them. Leaving them queued would carry them
    /// into the next prompt, where they arrive after that prompt's own position
    /// and, because this call clears the assembler, reopen a finished tool call
    /// as a second record instead of correcting the first.
    fn end_turn(
        &mut self,
        channel: &mut SessionChannel,
        events: &mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
        stop_reason: StopReason,
    ) {
        while let Ok(notification) = channel.updates.try_recv() {
            let outcome = self.recorder.record_update(&notification.update);
            self.settle_record(outcome);
            // Forwarded on the same best-effort terms as the loop's own updates:
            // the client may already be gone, which costs the stream but never
            // the record.
            let _ = events.try_send(Ok(PromptSessionEvent::SessionUpdate {
                update: notification.update,
            }));
        }
        let outcome = self.recorder.record_turn_end(stop_reason);
        self.settle_record(outcome);
    }

    /// Marks the session degraded when a recording attempt just broke its history.
    fn settle_record(&mut self, outcome: RecordOutcome) {
        let RecordOutcome::JustFailed { reason } = outcome else {
            return;
        };
        ora_debug!(
            session_id = %self.session.id,
            path = %self.recorder.path().display(),
            "session history stopped recording",
        );
        self.persist_session(|session, now| {
            session.with_history_state(HistoryState::Degraded { reason }, now)
        });
    }

    /// Streams Ora's recorded conversation to a client that asked to load it.
    ///
    /// Sends apply backpressure rather than failing fast: a long history is far
    /// larger than the event queue, and a slow consumer is not a disconnected one.
    async fn replay_recorded_history(
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
                let _ = events
                    .send(Err(runtime_internal(
                        "session_history_unreadable",
                        "session history could not be read",
                    )))
                    .await;
                return Replay::Unreadable;
            }
        };
        for line in history.lines {
            let event = match line.record {
                HistoryRecord::Update { update } => LoadSessionEvent::SessionUpdate { update },
                HistoryRecord::TurnEnded { stop_reason } => {
                    LoadSessionEvent::TurnEnded { stop_reason }
                }
                // Bookkeeping the conversation view has no place for. It stays in
                // the file, where the handoff renderer and a human can still see it.
                HistoryRecord::Meta(_)
                | HistoryRecord::AgentSwitched(_)
                | HistoryRecord::Gap { .. } => continue,
            };
            if events.send(Ok(event)).await.is_err() {
                return Replay::Abandoned;
            }
        }
        Replay::Delivered
    }

    /// Handles controls arriving while a registered session has no active operation.
    async fn handle_idle_control(&mut self, control: Option<SessionControl>) {
        match control {
            Some(SessionControl::Permission(permission)) => {
                if let Some(channel) = &self.channel {
                    let _ = channel
                        .connection
                        .client
                        .respond(
                            &permission.request_id,
                            &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                        )
                        .await;
                }
            }
            Some(SessionControl::UpdateOverflow) => self.unload().await,
            Some(SessionControl::ConnectionLost(_)) | None => self.mark_stopped(),
        }
    }

    /// Cancels the provider turn and settles every outstanding permission request.
    async fn cancel(
        &self,
        client: &AcpClient<ChildStdin>,
        permissions: &HashMap<String, (ora_contracts::acp::rpc::RequestId, Vec<String>)>,
    ) {
        ora_debug!(session_id = %self.session.id, pending_permissions = permissions.len(), "cancelling prompt");
        for (request_id, _) in permissions.values() {
            let _ = client
                .respond(
                    request_id,
                    &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                )
                .await;
        }
        let _ = client
            .notify(
                AGENT_METHOD_NAMES.session_cancel,
                &CancelNotification::new(self.session.agent_session_id.clone()),
            )
            .await;
    }

    /// Closes only this live ACP registration and preserves provider-owned history.
    async fn unload(&mut self) {
        if let Some(channel) = self.channel.take() {
            self.isolate_channel(channel).await;
        } else {
            self.mark_stopped();
        }
    }

    /// Detaches from the provider without recording any lifecycle change.
    ///
    /// Used only when the manager retires this actor, because it owns the row's
    /// next state and this actor's view of it is already out of date.
    async fn release(&mut self) {
        if let Some(channel) = self.channel.take() {
            self.close_provider_session(&channel).await;
        }
    }

    /// Detaches one routed session while leaving the shared CLI process available.
    async fn isolate_channel(&mut self, channel: SessionChannel) {
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
                        &CloseSessionRequest::new(self.session.agent_session_id.clone()),
                    ),
            )
            .await;
        }
    }

    /// Completes an interrupted load request with the connection-level failure.
    fn fail_load(
        &mut self,
        events: &mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
        error: BackendError,
    ) {
        let _ = events.try_send(Err(error));
        self.mark_stopped();
    }

    /// Completes an interrupted prompt request with the connection-level failure.
    fn fail_prompt(
        &mut self,
        events: &mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
        error: BackendError,
    ) {
        let _ = events.try_send(Err(error));
        self.mark_stopped();
    }

    /// Persists a stopped state after the provider session is detached or becomes unusable.
    fn mark_stopped(&mut self) {
        self.channel = None;
        self.persist_session(|session, now| session.with_status(SessionStatus::Stopped, now));
        ora_debug!(session_id = %self.session.id, "session marked stopped");
    }

    /// Applies one change to the stored session, refreshing the row first.
    ///
    /// The actor holds the snapshot it was built from, but switching agents and
    /// resuming history both rewrite fields it never learns about. Writing that
    /// snapshot back would silently revert them, so a lifecycle update reads the
    /// current row and changes only what it means to change.
    fn persist_session(&mut self, change: impl FnOnce(Session, i64) -> Session) {
        let now = self.clock.now_timestamp_millis();
        let current = self
            .repository
            .find_session(&self.session.id)
            .ok()
            .flatten()
            .unwrap_or_else(|| self.session.clone());
        self.session = change(current, now);
        let _ = self.repository.update_session(self.session.clone());
    }
}

/// Reports that the actor cannot accept a second operation while one is in flight.
fn session_busy() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::SessionBusy(EmptyErrorParams {}),
        "session already has an active operation",
    )
}

/// Reports that the requested permission no longer belongs to an active prompt.
fn permission_not_pending() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::PermissionRequestNotPending(EmptyErrorParams {}),
        "permission request is not pending",
    )
}
