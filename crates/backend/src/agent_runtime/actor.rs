use super::events::{
    drain_idle_events, drain_queued_prompt_events, settle_abandoned_session_response,
    settle_cancelled_prompt,
};
use super::handoff::prompt_for_agent;
use super::routing::{SessionControl, SessionEvent};
use super::scheduling::{ActiveInput, ActiveInputState};
use super::*;
use ora_acp::AcpClient;
use ora_contracts::SessionPermissionRequest;
use ora_contracts::acp::common::SessionId as AcpSessionId;
use ora_contracts::acp::literals::AGENT_METHOD_NAMES;
use ora_contracts::acp::notification::CancelNotification;
use ora_contracts::acp::permission::{RequestPermissionOutcome, RequestPermissionResponse};
use ora_contracts::acp::prompt::{PromptRequest, PromptResponse, StopReason};
use ora_contracts::acp::session::{
    CloseSessionRequest, CloseSessionResponse, ConfigOptionUpdate,
    LoadSessionRequest as AcpLoadSessionRequest, LoadSessionResponse, SessionUpdate,
};
use ora_history::HistoryRecord;
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
                    // Residual events belong to the previous provider turn. Consume the current
                    // queue snapshot before accepting a new command so they cannot cross turns.
                    drain_idle_events(&channel.connection.client, &mut channel.events).await;
                    if let Ok(control) = channel.controls.try_recv() {
                        self.handle_idle_control(Some(control)).await;
                        continue;
                    }
                    tokio::select! {
                        biased;
                        control = channel.controls.recv() => {
                            self.handle_idle_control(control).await;
                            continue;
                        }
                        command = self.commands.recv() => {
                            drain_idle_events(
                                &channel.connection.client,
                                &mut channel.events,
                            )
                            .await;
                            command
                        }
                        event = channel.events.recv() => {
                            let Some(event) = event else {
                                self.mark_stopped();
                                continue;
                            };
                            super::events::settle_idle_event(&channel.connection.client, event).await;
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
            .open_session_channel(&self.session.agent_session_id, self.session.id.as_ref())
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

    /// Completes provider load only after its ordered response fence follows all prior events.
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
        let pending = match client
            .start_session_request::<_, LoadSessionResponse>(
                AcpSessionId::new(self.session.agent_session_id.clone()),
                AGENT_METHOD_NAMES.session_load,
                &request,
            )
            .await
        {
            Ok(pending) => pending,
            Err(error) => {
                let _ = events.try_send(Err(map_acp_error(error)));
                self.isolate_channel(channel).await;
                return;
            }
        };
        let deadline = tokio::time::sleep(SESSION_SETUP_TIMEOUT);
        tokio::pin!(deadline);
        let mut input_state = ActiveInputState::default();
        loop {
            let input = tokio::select! {
                // A response already accepted by the FIFO is more useful than a deadline that
                // became ready at the same time; preceding events have already been consumed.
                biased;
                input = input_state.recv(
                    &mut channel.events,
                    &mut channel.controls,
                    &mut self.commands,
                ) => input,
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
            };
            match input {
                ActiveInput::Event(SessionEvent::Update(_)) => {
                    // The agent is reciting history Ora already owns. Draining it keeps the
                    // queue clear and proves the provider is still working.
                    deadline
                        .as_mut()
                        .reset(Instant::now() + SESSION_SETUP_TIMEOUT);
                }
                ActiveInput::Event(SessionEvent::Permission(permission)) => {
                    let _ = client
                        .respond(
                            &permission.request_id,
                            &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                        )
                        .await;
                    let _ = events.try_send(Err(runtime_internal(
                        "agent_protocol_error",
                        "permission request during session/load is unsupported",
                    )));
                    self.isolate_channel(channel).await;
                    return;
                }
                ActiveInput::Event(SessionEvent::Response(response)) => {
                    if !pending.matches_response(&response) {
                        continue;
                    }
                    match pending.finish(response) {
                        Ok(response) => {
                            ora_debug!(session_id = %self.session.id, "session/load completed");
                            // `session/load` reports configuration options in its reply rather
                            // than as an update, so they stay ahead of the recorded replay.
                            if let Some(config_options) = response.config_options
                                && events
                                    .send(Ok(LoadSessionEvent::SessionUpdate {
                                        update: SessionUpdate::ConfigOptionUpdate(
                                            ConfigOptionUpdate::new(config_options),
                                        ),
                                    }))
                                    .await
                                    .is_err()
                            {
                                self.isolate_channel(channel).await;
                                return;
                            }
                            match self.replay_recorded_history(&events).await {
                                Replay::Delivered
                                    if events
                                        .send(Ok(LoadSessionEvent::Completed))
                                        .await
                                        .is_ok() =>
                                {
                                    self.channel = Some(channel);
                                }
                                // A replay that did not finish leaves the client without the
                                // conversation it asked for, so the registration goes with it.
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
                ActiveInput::Control(SessionControl::ConnectionLost(error)) => {
                    self.fail_load(&events, error);
                    return;
                }
                ActiveInput::Control(SessionControl::QueueOverflow) => {
                    let _ = events.try_send(Err(runtime_internal(
                        "agent_event_overflow",
                        "session event queue overflowed",
                    )));
                    self.isolate_channel(channel).await;
                    return;
                }
                ActiveInput::EventsClosed | ActiveInput::ControlsClosed => {
                    self.fail_load(&events, runtime_unavailable());
                    return;
                }
                ActiveInput::Command(RuntimeCommand::Cancel {
                    operation_id: cancelled,
                }) if cancelled == operation_id => {
                    self.cancel(&client, &HashMap::new()).await;
                    let _ = timeout(
                        CANCELLATION_GRACE,
                        settle_abandoned_session_response(&mut channel, &client, pending),
                    )
                    .await;
                    self.isolate_channel(channel).await;
                    return;
                }
                ActiveInput::Command(RuntimeCommand::Stop { response }) => {
                    self.cancel(&client, &HashMap::new()).await;
                    self.isolate_channel(channel).await;
                    let _ = response.send(Ok(StopSessionResponse {
                        session: contract_session(self.session.clone()),
                    }));
                    return;
                }
                ActiveInput::Command(
                    RuntimeCommand::Prompt { accepted, .. } | RuntimeCommand::Load { accepted, .. },
                ) => {
                    let _ = accepted.send(Err(session_busy()));
                }
                ActiveInput::Command(RuntimeCommand::RespondToPermission { response, .. }) => {
                    let _ = response.send(Err(permission_not_pending()));
                }
                ActiveInput::Command(RuntimeCommand::Cancel { .. }) => {}
                ActiveInput::CommandsClosed => {
                    self.cancel(&client, &HashMap::new()).await;
                    self.isolate_channel(channel).await;
                    return;
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
        let client = channel.connection.client.clone();
        // Catch events that arrived after the previous operation ended but before this command
        // was accepted. Setup updates in `pending_updates` are intentional and stay separate.
        drain_idle_events(&client, &mut channel.events).await;
        if let Ok(control) = channel.controls.try_recv() {
            match control {
                SessionControl::QueueOverflow => {
                    let _ = events.try_send(Err(runtime_internal(
                        "agent_event_overflow",
                        "session event queue overflowed",
                    )));
                    self.isolate_channel(channel).await;
                    return;
                }
                SessionControl::ConnectionLost(error) => {
                    self.fail_prompt(&events, error);
                    return;
                }
            }
        }
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
        let content_count = prompt.len();
        // Built before the prompt is recorded, so the transcript handed to a new
        // agent describes the conversation up to this turn rather than including it.
        let handoff_carried = self.handoff_pending;
        let sent = prompt_for_agent(self, &prompt);
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
        let pending = match client
            .start_session_request::<_, PromptResponse>(
                AcpSessionId::new(self.session.agent_session_id.clone()),
                AGENT_METHOD_NAMES.session_prompt,
                &request,
            )
            .await
        {
            Ok(pending) => pending,
            Err(error) => {
                self.end_turn(StopReason::Cancelled);
                let _ = events.try_send(Err(map_acp_error(error)));
                self.isolate_channel(channel).await;
                return;
            }
        };
        let mut permissions = HashMap::new();
        let mut input_state = ActiveInputState::default();
        loop {
            match input_state
                .recv(
                    &mut channel.events,
                    &mut channel.controls,
                    &mut self.commands,
                )
                .await
            {
                ActiveInput::Event(SessionEvent::Update(update)) => {
                    // Record before forwarding: a client that drops mid-turn must not also cost
                    // the durable record of what the provider produced.
                    let outcome = self.recorder.record_update(&update.update);
                    self.settle_record(outcome);
                    if events
                        .try_send(Ok(PromptSessionEvent::SessionUpdate {
                            update: update.update,
                        }))
                        .is_err()
                    {
                        self.end_turn(StopReason::Cancelled);
                        self.cancel(&client, &permissions).await;
                        self.isolate_channel(channel).await;
                        return;
                    }
                }
                ActiveInput::Event(SessionEvent::Permission(permission)) => {
                    let public_id = permission.request_id.to_string();
                    let option_ids = permission
                        .request
                        .options
                        .iter()
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
                        self.end_turn(StopReason::Cancelled);
                        self.cancel(&client, &permissions).await;
                        self.isolate_channel(channel).await;
                        return;
                    }
                }
                ActiveInput::Event(SessionEvent::Response(response)) => {
                    if !pending.matches_response(&response) {
                        continue;
                    }
                    match pending.finish(response) {
                        Ok(response) => {
                            ora_debug!(session_id = %self.session.id, stop_reason = ?response.stop_reason, "prompt completed");
                            self.end_turn(response.stop_reason);
                            if events
                                .try_send(Ok(PromptSessionEvent::Completed {
                                    stop_reason: response.stop_reason,
                                }))
                                .is_ok()
                            {
                                self.channel = Some(channel);
                            } else {
                                self.isolate_channel(channel).await;
                            }
                        }
                        Err(error) => {
                            let reusable = matches!(&error, ora_acp::AcpError::RequestFailed(_));
                            ora_debug!(session_id = %self.session.id, error = %error, reusable = reusable, "prompt failed");
                            self.end_turn(StopReason::Cancelled);
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
                ActiveInput::Control(SessionControl::ConnectionLost(error)) => {
                    self.end_turn(StopReason::Cancelled);
                    self.fail_prompt(&events, error);
                    return;
                }
                ActiveInput::Control(SessionControl::QueueOverflow) => {
                    self.end_turn(StopReason::Cancelled);
                    self.cancel(&client, &permissions).await;
                    let _ = events.try_send(Err(runtime_internal(
                        "agent_event_overflow",
                        "session event queue overflowed",
                    )));
                    self.isolate_channel(channel).await;
                    return;
                }
                ActiveInput::EventsClosed | ActiveInput::ControlsClosed => {
                    self.end_turn(StopReason::Cancelled);
                    self.fail_prompt(&events, runtime_unavailable());
                    return;
                }
                ActiveInput::Command(RuntimeCommand::RespondToPermission { request, response }) => {
                    let result = respond_permission(&client, request, &mut permissions).await;
                    let _ = response.send(result);
                }
                ActiveInput::Command(RuntimeCommand::Cancel {
                    operation_id: cancelled,
                }) if cancelled == operation_id => {
                    self.cancel(&client, &permissions).await;
                    let settled = timeout(
                        CANCELLATION_GRACE,
                        settle_cancelled_prompt(self, &mut channel, &client, pending, &events),
                    )
                    .await;
                    match settled {
                        Ok(Some(Ok(_))) | Ok(Some(Err(ora_acp::AcpError::RequestFailed(_)))) => {
                            self.end_turn(StopReason::Cancelled);
                            self.channel = Some(channel);
                        }
                        Ok(Some(Err(_))) | Ok(None) | Err(_) => {
                            drain_queued_prompt_events(self, &mut channel, &client, &events).await;
                            self.end_turn(StopReason::Cancelled);
                            self.isolate_channel(channel).await;
                        }
                    }
                    return;
                }
                ActiveInput::Command(RuntimeCommand::Stop { response }) => {
                    self.cancel(&client, &permissions).await;
                    self.end_turn(StopReason::Cancelled);
                    self.isolate_channel(channel).await;
                    let _ = response.send(Ok(StopSessionResponse {
                        session: contract_session(self.session.clone()),
                    }));
                    return;
                }
                ActiveInput::Command(
                    RuntimeCommand::Prompt { accepted, .. } | RuntimeCommand::Load { accepted, .. },
                ) => {
                    let _ = accepted.send(Err(session_busy()));
                }
                ActiveInput::Command(RuntimeCommand::Cancel { .. }) => {}
                ActiveInput::CommandsClosed => {
                    self.cancel(&client, &permissions).await;
                    self.end_turn(StopReason::Cancelled);
                    self.isolate_channel(channel).await;
                    return;
                }
            }
        }
    }

    /// Closes the recorded turn after the ordered event consumer has settled its events.
    fn end_turn(&mut self, stop_reason: StopReason) {
        let outcome = self.recorder.record_turn_end(stop_reason);
        self.settle_record(outcome);
    }

    /// Marks the session degraded when a recording attempt just broke its history.
    pub(super) fn settle_record(&mut self, outcome: RecordOutcome) {
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
            Some(SessionControl::QueueOverflow) => self.unload().await,
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
