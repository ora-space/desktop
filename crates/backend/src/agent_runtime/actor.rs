use super::connection::AgentAcpClient;
use super::events::{
    drain_idle_events, drain_queued_prompt_events, settle_abandoned_session_response,
    settle_cancelled_prompt,
};
use super::handoff::{AgentPrompt, prompt_for_agent};
use super::replay::recorded_replay;
use super::routing::{SessionControl, SessionEvent};
use super::scheduling::{ActiveInput, ActiveInputState};
use super::session_followers::SessionFollowers;
use super::title_acquisition::PollAttempt;
use super::*;
#[path = "title_polling.rs"]
mod title_polling;
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::CancelNotification;
use agent_client_protocol_schema::v1::SessionId as AcpSessionId;
use agent_client_protocol_schema::v1::{
    CloseSessionRequest, CloseSessionResponse, ConfigOptionUpdate,
    LoadSessionRequest as AcpLoadSessionRequest, LoadSessionResponse, SessionUpdate,
};
use agent_client_protocol_schema::v1::{PromptRequest, PromptResponse, StopReason};
use agent_client_protocol_schema::v1::{RequestPermissionOutcome, RequestPermissionResponse};
use ora_logging::{ora_debug, ora_warn};
use tokio::time::{Instant, timeout};

/// How far replaying Ora's record got before it stopped.
///
/// Only `Delivered` may complete the load stream. The other two are kept apart because
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
            let command_sender = self.command_sender.clone();
            let command = match self.channel.as_mut() {
                Some(channel) => {
                    // Residual events belong to the previous provider turn. Consume the current
                    // queue snapshot before accepting a new command so they cannot cross turns.
                    drain_idle_events(
                        &channel.connection.client,
                        &mut channel.events,
                        &command_sender,
                    )
                    .await;
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
                                &command_sender,
                            )
                            .await;
                            command
                        }
                        event = channel.events.recv() => {
                            let Some(event) = event else {
                                self.mark_stopped();
                                continue;
                            };
                            if let SessionEvent::Update(update) = &event
                                && let Some(command_sender) = command_sender.upgrade()
                            {
                                let _ = command_sender.send(RuntimeCommand::TitleUpdate {
                                    update: Box::new(update.update.clone()),
                                });
                            }
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
                    self.run_load(operation_id, events, accepted).await;
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
                RuntimeCommand::AgentProcessReplaced { agent } => {
                    self.detach_replaced_agent(&agent);
                }
                RuntimeCommand::RespondToPermission { response, .. } => {
                    let _ = response.send(Err(permission_not_pending()));
                }
                RuntimeCommand::Stop { response } => {
                    self.title_acquisition.close();
                    self.unload().await;
                    let _ = response.send(Ok(StopSessionResponse {
                        session: contract_session(self.session.clone()),
                    }));
                }
                RuntimeCommand::CancelActivePrompt => {}
                RuntimeCommand::Cancel { .. } => {}
                RuntimeCommand::PreemptTitlePolling { response } => {
                    let _ = response.send(());
                }
                RuntimeCommand::AdoptUserTitle { title, response } => {
                    self.adopt_user_title(title);
                    let _ = response.send(());
                }
                RuntimeCommand::TitlePoll { attempt } => {
                    self.run_title_poll(attempt).await;
                }
                RuntimeCommand::TitleUpdate { update } => {
                    self.observe_session_update(&update);
                }
            }
        }
    }

    /// Re-registers a stopped session and streams provider history without replacing the process.
    ///
    /// The admission signal is only sent after the Running row is persisted:
    /// an aggregate-deletion cascade that runs after `accepted` resolves must
    /// observe the Running session and refuse the delete, otherwise it could
    /// remove the checkout while provider setup is still using it.
    async fn run_load(
        &mut self,
        operation_id: u64,
        events: mpsc::Sender<Result<LoadSessionEvent, BackendError>>,
        accepted: oneshot::Sender<Result<(), BackendError>>,
    ) {
        self.unload().await;
        let running = match self.repository.update_session_status(
            &self.session.id,
            SessionStatus::Running,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(session) => session,
            Err(_) => {
                // The session (or its task/project) is no longer visible — a
                // deletion won the race. Reject the admission itself so no
                // caller proceeds against a removed checkout.
                let _ = accepted.send(Err(session_not_found(self.session.id.as_ref())));
                return;
            }
        };
        let _ = accepted.send(Ok(()));
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
                ActiveInput::Event(SessionEvent::Update(update)) => {
                    self.observe_session_update(&update.update);
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
                        settle_abandoned_session_response(self, &mut channel, &client, pending),
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
                ActiveInput::Command(RuntimeCommand::CancelActivePrompt) => {}
                // The Effect barrier is what makes this unreachable while an operation is in
                // flight: a consumer is only restarted after its plugin reported every turn
                // finished. A replacement that still raced in leaves this request unanswered, and
                // this loop's existing failure handling ends the operation and stops the session —
                // the same repair the idle path performs, arrived at the slower way.
                ActiveInput::Command(RuntimeCommand::AgentProcessReplaced { .. }) => {}
                ActiveInput::Command(
                    RuntimeCommand::Prompt { accepted, .. } | RuntimeCommand::Load { accepted, .. },
                ) => {
                    let _ = accepted.send(Err(session_busy()));
                }
                ActiveInput::Command(RuntimeCommand::RespondToPermission { response, .. }) => {
                    let _ = response.send(Err(permission_not_pending()));
                }
                ActiveInput::Command(RuntimeCommand::PreemptTitlePolling { response }) => {
                    let _ = response.send(());
                }
                ActiveInput::Command(RuntimeCommand::AdoptUserTitle { title, response }) => {
                    self.adopt_user_title(title);
                    let _ = response.send(());
                }
                ActiveInput::Command(RuntimeCommand::Cancel { .. }) => {}
                ActiveInput::Command(RuntimeCommand::TitlePoll {
                    attempt: PollAttempt::First,
                }) => {
                    self.title_acquisition.finish_attempt(PollAttempt::First);
                }
                ActiveInput::Command(RuntimeCommand::TitlePoll {
                    attempt: PollAttempt::Final,
                }) => {
                    self.title_acquisition.finish_attempt(PollAttempt::Final);
                    self.title_acquisition.close();
                }
                ActiveInput::Command(RuntimeCommand::TitleUpdate { update }) => {
                    self.observe_session_update(&update);
                }
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
        drain_idle_events(&client, &mut channel.events, &self.command_sender).await;
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
            self.observe_session_update(&notification.update);
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
        let AgentPrompt {
            blocks,
            settles_handoff,
        } = prompt_for_agent(self, &prompt);
        let outcome = self.recorder.record_prompt(&prompt);
        let stopped_recording = matches!(outcome, RecordOutcome::JustFailed { .. });
        self.settle_record(outcome);
        if stopped_recording {
            // A turn already streaming is allowed to finish, because the agent's
            // work is real whether or not the file kept it. This one has not
            // started: nothing is lost by refusing it, and running it would put
            // the conversation somewhere the record cannot follow.
            let _ = events.try_send(Err(history_degraded()));
            self.channel = Some(channel);
            return;
        }
        let request = PromptRequest::new(self.session.agent_session_id.clone(), blocks);
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
                // The request never reached the agent, so a transcript this prompt
                // was carrying is still owed and stays owed — in memory and, since
                // no delivery was recorded, across a restart as well.
                self.end_turn(StopReason::Cancelled);
                let _ = events.try_send(Err(map_acp_error(error)));
                self.isolate_channel(channel).await;
                return;
            }
        };
        if settles_handoff {
            // Accepting the request is the last thing Ora can observe about delivery:
            // past it the frame is on the agent's stdin and what the agent does with
            // it is unobservable. Treating that as delivered keeps a connection lost
            // mid-turn from re-injecting the whole conversation into an agent that
            // already holds it — the transcript's preamble, which tells its reader
            // the work belongs to a *different* agent, would be false if it did.
            //
            // Recording it can itself fail, which degrades the session and leaves no
            // delivery line behind. The next actor then reads the binding as still
            // owing a handoff and sends it again, the harmless direction to be wrong in.
            self.handoff_pending = false;
            let outcome = self
                .recorder
                .record_handoff_delivered(self.session.agent_session_id.clone());
            self.settle_record(outcome);
        }
        let mut permissions = HashMap::new();
        let mut followers = SessionFollowers::new();
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
                    self.observe_session_update(&update.update);
                    let update = update.update;
                    let outcome = self.recorder.record_update(&update);
                    self.settle_record(outcome);
                    followers.send_update(&update);
                    if events
                        .try_send(Ok(PromptSessionEvent::SessionUpdate { update }))
                        .is_err()
                    {
                        self.end_turn(StopReason::Cancelled);
                        followers.finish(StopReason::Cancelled);
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
                    // Ora has no user-configurable approval policy yet, so every request is
                    // granted through the same `respond_permission` path a real user's choice
                    // would take, instead of being left pending for
                    // `RuntimeCommand::RespondToPermission`. A future policy can gate this
                    // auto-response and leave the request in `permissions` for the frontend to
                    // answer instead.
                    let auto_option_id = pick_auto_allow_option(&permission.request.options)
                        .map(|option| option.option_id.to_string());
                    permissions.insert(public_id.clone(), (permission.request_id, option_ids));
                    let Some(option_id) = auto_option_id else {
                        ora_warn!(session_id = %self.session.id, request_id = %public_id, "permission request offered no allow option");
                        self.end_turn(StopReason::Cancelled);
                        followers.finish(StopReason::Cancelled);
                        self.cancel(&client, &permissions).await;
                        self.isolate_channel(channel).await;
                        return;
                    };
                    let auto_response = respond_permission(
                        &client,
                        RespondToPermissionRequest {
                            session_id: self.session.id.to_string(),
                            permission_request_id: public_id,
                            option_id,
                        },
                        &mut permissions,
                    )
                    .await;
                    if let Err(error) = auto_response {
                        ora_warn!(session_id = %self.session.id, error = %error, "failed to auto-allow permission request");
                        self.end_turn(StopReason::Cancelled);
                        followers.finish(StopReason::Cancelled);
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
                            followers.finish(response.stop_reason);
                            self.maybe_start_title_acquisition(response.stop_reason);
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
                            followers.finish(StopReason::Cancelled);
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
                    followers.finish(StopReason::Cancelled);
                    self.fail_prompt(&events, error);
                    return;
                }
                ActiveInput::Control(SessionControl::QueueOverflow) => {
                    self.end_turn(StopReason::Cancelled);
                    followers.finish(StopReason::Cancelled);
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
                    followers.finish(StopReason::Cancelled);
                    self.fail_prompt(&events, runtime_unavailable());
                    return;
                }
                ActiveInput::Command(RuntimeCommand::RespondToPermission { request, response }) => {
                    let result = respond_permission(&client, request, &mut permissions).await;
                    let _ = response.send(result);
                }
                ActiveInput::Command(RuntimeCommand::Cancel {
                    operation_id: cancelled,
                }) if followers.remove(cancelled) => {}
                ActiveInput::Command(command)
                    if matches!(&command, RuntimeCommand::CancelActivePrompt)
                        || matches!(
                            &command,
                            RuntimeCommand::Cancel {
                                operation_id: cancelled
                            } if *cancelled == operation_id
                        ) =>
                {
                    let notify_owner = matches!(command, RuntimeCommand::CancelActivePrompt);
                    self.cancel(&client, &permissions).await;
                    let settled = timeout(
                        CANCELLATION_GRACE,
                        settle_cancelled_prompt(self, &mut channel, &client, pending, &events),
                    )
                    .await;
                    let reusable = matches!(
                        settled,
                        Ok(Some(Ok(_))) | Ok(Some(Err(ora_acp::AcpError::RequestFailed(_))))
                    );
                    if !reusable {
                        drain_queued_prompt_events(self, &mut channel, &client, &events).await;
                    }
                    self.end_turn(StopReason::Cancelled);
                    followers.finish(StopReason::Cancelled);
                    let owner_notified = !notify_owner
                        || events
                            .try_send(Ok(PromptSessionEvent::Completed {
                                stop_reason: StopReason::Cancelled,
                            }))
                            .is_ok();
                    if reusable && owner_notified {
                        self.channel = Some(channel);
                    } else {
                        self.isolate_channel(channel).await;
                    }
                    return;
                }
                ActiveInput::Command(RuntimeCommand::Stop { response }) => {
                    self.cancel(&client, &permissions).await;
                    self.end_turn(StopReason::Cancelled);
                    followers.finish(StopReason::Cancelled);
                    self.isolate_channel(channel).await;
                    let _ = response.send(Ok(StopSessionResponse {
                        session: contract_session(self.session.clone()),
                    }));
                    return;
                }
                ActiveInput::Command(RuntimeCommand::Prompt { accepted, .. }) => {
                    let _ = accepted.send(Err(session_busy()));
                }
                ActiveInput::Command(RuntimeCommand::Load {
                    operation_id,
                    events,
                    accepted,
                }) => {
                    // Capture the durable cutoff, the in-progress pending records, and the live
                    // follower registration atomically (no await), then let the follower's relay
                    // task stream the merged prefix before live events. The actor returns to its
                    // select loop immediately, so a slow view can never backpressure the prompt.
                    let cutoff = self.recorder.durable_bytes();
                    let pending = self.recorder.pending_records();
                    if accepted.send(Ok(())).is_ok() {
                        followers.insert(
                            operation_id,
                            events,
                            self.sessions_root.clone(),
                            self.session.id.to_string(),
                            cutoff,
                            pending,
                        );
                    }
                }
                ActiveInput::Command(RuntimeCommand::PreemptTitlePolling { response }) => {
                    let _ = response.send(());
                }
                ActiveInput::Command(RuntimeCommand::AdoptUserTitle { title, response }) => {
                    self.adopt_user_title(title);
                    let _ = response.send(());
                }
                ActiveInput::Command(RuntimeCommand::Cancel { .. }) => {}
                ActiveInput::Command(RuntimeCommand::CancelActivePrompt) => {}
                // The Effect barrier is what makes this unreachable while an operation is in
                // flight: a consumer is only restarted after its plugin reported every turn
                // finished. A replacement that still raced in leaves this request unanswered, and
                // this loop's existing failure handling ends the operation and stops the session —
                // the same repair the idle path performs, arrived at the slower way.
                ActiveInput::Command(RuntimeCommand::AgentProcessReplaced { .. }) => {}
                ActiveInput::Command(RuntimeCommand::TitlePoll {
                    attempt: PollAttempt::First,
                }) => {
                    self.title_acquisition.finish_attempt(PollAttempt::First);
                }
                ActiveInput::Command(RuntimeCommand::TitlePoll {
                    attempt: PollAttempt::Final,
                }) => {
                    self.title_acquisition.finish_attempt(PollAttempt::Final);
                    self.title_acquisition.close();
                }
                ActiveInput::Command(RuntimeCommand::TitleUpdate { update }) => {
                    self.observe_session_update(&update);
                }
                ActiveInput::CommandsClosed => {
                    self.cancel(&client, &permissions).await;
                    self.end_turn(StopReason::Cancelled);
                    followers.finish(StopReason::Cancelled);
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
        self.persist_session_history_state(HistoryState::Degraded { reason });
    }

    /// Streams Ora's recorded conversation to a client that loaded it.
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
        for event in recorded_replay(history) {
            if events.send(Ok(event)).await.is_err() {
                return Replay::Abandoned;
            }
        }
        Replay::Delivered
    }

    /// Handles controls arriving while a registered session has no active operation.
    async fn handle_idle_control(&mut self, control: Option<SessionControl>) {
        match control {
            Some(SessionControl::QueueOverflow) => {
                self.title_acquisition.close();
                self.unload().await;
            }
            Some(SessionControl::ConnectionLost(_)) | None => self.mark_stopped(),
        }
    }

    /// Cancels the provider turn and settles every outstanding permission request.
    async fn cancel(
        &self,
        client: &AgentAcpClient,
        permissions: &HashMap<String, (agent_client_protocol_schema::v1::RequestId, Vec<String>)>,
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
    async fn release(&mut self) {
        self.title_acquisition.close();
        if let Some(channel) = self.channel.take() {
            self.close_provider_session(&channel).await;
        }
    }

    /// Detaches one routed session while leaving the shared CLI process available.
    async fn isolate_channel(&mut self, channel: SessionChannel) {
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

    /// Drops the live registration when this session's agent process was replaced under it.
    ///
    /// The provider session died with the process, so the channel is dropped without `session/close`
    /// — that call would only ask a fresh agent to close an id it has never heard of. Detaching is
    /// enough to repair the session rather than end it: a prompt is refused while no channel is
    /// held, and the load that establishes one calls `session/load`, which is exactly the
    /// re-establishment the replaced process needs. Without this the actor would keep a channel it
    /// believes is live and prompt against a session id the new process cannot resolve.
    fn detach_replaced_agent(&mut self, agent: &ora_domain::AgentRef) {
        if self.session.agent_ref != *agent || self.channel.is_none() {
            return;
        }
        ora_debug!(
            session_id = %self.session.id,
            agent = %agent,
            "detaching session after its agent process was replaced",
        );
        self.mark_stopped();
    }

    /// Persists a stopped state after the provider session is detached or becomes unusable.
    fn mark_stopped(&mut self) {
        self.channel = None;
        self.title_acquisition.close();
        self.persist_session_status(SessionStatus::Stopped);
        ora_debug!(session_id = %self.session.id, "session marked stopped");
    }

    /// Persists lifecycle status and refreshes the actor snapshot from the single-column result.
    fn persist_session_status(&mut self, status: SessionStatus) {
        match self.repository.update_session_status(
            &self.session.id,
            status,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(session) => self.session = session,
            Err(error) => ora_warn!(
                session_id = %self.session.id,
                error = %error,
                "failed to persist session lifecycle status",
            ),
        }
    }

    /// Persists history state without allowing an older actor snapshot to overwrite title fields.
    fn persist_session_history_state(&mut self, history_state: HistoryState) {
        match self.repository.update_session_history_state(
            &self.session.id,
            &history_state,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(session) => self.session = session,
            Err(error) => ora_warn!(
                session_id = %self.session.id,
                error = %error,
                "failed to persist session history state",
            ),
        }
    }
}

#[cfg(test)]
impl Drop for RuntimeActor {
    /// Signals the test harness after the actor has released all of its dependencies.
    fn drop(&mut self) {
        if let Some(exit_probe) = self.exit_probe.take() {
            let _ = exit_probe.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::connection::AgentSource;
    use super::*;
    use crate::agent_runtime::connection::ConnectionSupervisor;
    use crate::agent_runtime::title_acquisition::TitleAcquisition;
    use crate::app_event::AppEventHub;
    use crate::clock::SystemClock;
    use crate::plugin::PluginApi;
    use crate::user_config::UserConfigApi;
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, RepositoryPool, default_migration_catalog,
    };
    use ora_domain::{
        AgentRef, AuditFields, PluginId, SessionId, SessionStatus, SessionTitle, WorkspaceId,
    };
    use ora_scheduler::Scheduler;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::timeout;

    /// Opens one migrated pool for the plugin host's database-backed collaborators.
    fn test_repository_pool(root: &Path) -> RepositoryPool {
        DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(root.join("test.sqlite")),
                &default_migration_catalog().expect("build migration catalog"),
            )
            .expect("create repository pool")
    }

    /// Builds a plugin host over an empty package root for these CLI-only supervisor tests.
    fn test_plugin_host(pool: &RepositoryPool, root: &Path) -> Arc<PluginApi> {
        Arc::new(
            PluginApi::open(
                pool.clone(),
                root.to_path_buf(),
                PathBuf::from("deno"),
                SystemClock,
                AppEventHub::new().publisher(),
                Arc::new(UserConfigApi::new(pool.clone())),
            )
            .expect("open plugin host"),
        )
    }

    /// Names one installed agent package the supervisor fixtures bind their sessions to.
    fn test_agent_source() -> (AgentRef, AgentSource) {
        (
            AgentRef::parse("ora-space.codex").expect("agent identity"),
            AgentSource {
                plugin_id: PluginId::new("official", "ora-space.codex").expect("plugin id"),
                package_name: "ora-space.codex".to_string(),
            },
        )
    }

    /// Verifies dropping the manager's last sender lets the actor task terminate and release its dependencies.
    #[tokio::test]
    async fn actor_exits_after_command_sender_is_dropped() {
        let temporary = TempDir::new().expect("create actor test directory");
        let pool = test_repository_pool(temporary.path());
        let scheduler = Scheduler::new(chrono_tz::UTC);
        let (agent_ref, agent_source) = test_agent_source();
        let connection = ConnectionSupervisor::start(
            agent_ref.clone(),
            agent_source,
            test_plugin_host(&pool, temporary.path()),
            pool.clone(),
            temporary.path().to_path_buf(),
            SystemClock,
        );
        let recorder = super::super::history::SessionRecorder::open(
            &temporary.path().join("sessions"),
            "session-1",
            0,
            &ora_domain::HistoryState::Writable,
            super::super::history::LocalHistoryClock,
        )
        .expect("open actor recorder");
        let session = ora_domain::Session::new(
            SessionId::new("session-1"),
            WorkspaceId::new("workspace-1"),
            agent_ref,
            "provider-session-1",
            SessionStatus::Stopped,
            AuditFields::new(0, 0, false),
        );
        let (commands, command_receiver) = mpsc::unbounded_channel();
        let command_sender = commands.downgrade();
        let (exit_sender, exit) = oneshot::channel();
        let actor = RuntimeActor {
            session,
            cwd: temporary.path().to_path_buf(),
            repository: ora_db::SqliteSessionRepository::new(pool),
            clock: SystemClock,
            connection,
            channel: None,
            commands: command_receiver,
            recorder,
            sessions_root: temporary.path().join("sessions"),
            handoff_pending: false,
            scheduler: scheduler.clone(),
            app_events: AppEventHub::new().publisher(),
            title_acquisition: TitleAcquisition::disabled(),
            command_sender,
            exit_probe: Some(exit_sender),
        };
        let actor_task = tokio::spawn(actor.run());

        drop(commands);
        timeout(Duration::from_secs(1), exit)
            .await
            .expect("actor should drop after its command channel closes")
            .expect("actor drop probe should remain connected");
        actor_task.await.expect("actor task should exit cleanly");
        scheduler.shutdown().await;
    }

    /// Verifies a user-chosen title locks acquisition so a later agent title is ignored.
    #[tokio::test]
    async fn user_rename_locks_title_against_later_agent_updates() {
        let temporary = TempDir::new().expect("create actor test directory");
        let pool = test_repository_pool(temporary.path());
        let scheduler = Scheduler::new(chrono_tz::UTC);
        let (agent_ref, agent_source) = test_agent_source();
        let connection = ConnectionSupervisor::start(
            agent_ref.clone(),
            agent_source,
            test_plugin_host(&pool, temporary.path()),
            pool.clone(),
            temporary.path().to_path_buf(),
            SystemClock,
        );
        let recorder = super::super::history::SessionRecorder::open(
            &temporary.path().join("sessions"),
            "session-1",
            0,
            &ora_domain::HistoryState::Writable,
            super::super::history::LocalHistoryClock,
        )
        .expect("open actor recorder");
        let session = ora_domain::Session::new(
            SessionId::new("session-1"),
            WorkspaceId::new("workspace-1"),
            agent_ref,
            "provider-session-1",
            SessionStatus::Stopped,
            AuditFields::new(0, 0, false),
        );
        let (commands, command_receiver) = mpsc::unbounded_channel();
        let command_sender = commands.downgrade();
        let mut actor = RuntimeActor {
            session,
            cwd: temporary.path().to_path_buf(),
            repository: ora_db::SqliteSessionRepository::new(pool),
            clock: SystemClock,
            connection,
            channel: None,
            commands: command_receiver,
            recorder,
            sessions_root: temporary.path().join("sessions"),
            handoff_pending: false,
            scheduler: scheduler.clone(),
            app_events: AppEventHub::new().publisher(),
            title_acquisition: TitleAcquisition::awaiting_first_prompt(true),
            command_sender,
            exit_probe: None,
        };
        let user_title = SessionTitle::parse("User title").expect("valid user title");
        actor.adopt_user_title(user_title.clone());
        actor.persist_agent_title("Agent title");
        assert_eq!(actor.session.title.as_ref(), Some(&user_title));
        assert!(!actor.title_acquisition.accepts_title());
        drop(commands);
        drop(actor);
        scheduler.shutdown().await;
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
