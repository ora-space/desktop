//! One prompt turn: admission, provider streaming, and the end it reports.
//!
//! Kept apart from the actor's command loop so the turn's many termination paths stay readable
//! together: each one records why the turn ended before its owner can observe it settled.

use super::events::{drain_queued_prompt_events, settle_cancelled_prompt};
use super::handoff::{AgentPrompt, prompt_for_agent, rebuild_prompt};
use super::prompt_liveness::PromptLiveness;
use super::prompt_retry::{StalledPrompt, retry_stalled_prompt};
use super::scheduling::{ActiveInput, ActiveInputState};
use super::session_followers::{FollowerOutput, SessionFollowers};
use super::title_acquisition::PollAttempt;
use super::tool_timing::ToolTimings;
use super::*;
use crate::attach::LoadRestoration;
use agent_client_protocol_schema::v1::SessionId as AcpSessionId;
use agent_client_protocol_schema::v1::{
    AGENT_METHOD_NAMES, PromptRequest, PromptResponse, SessionConfigKind, SessionConfigOption,
    SessionConfigOptionCategory, StopReason,
};
use ora_logging::{ora_debug, ora_warn};
use tokio::time::timeout;

/// Whether one provider update is work the user would see or a tool already ran.
///
/// `Plan` is liveness activity, not output: it carries no assistant text and no tool side effect,
/// so a failed prompt that only planned may still be retried safely.
fn produced_output(update: &SessionUpdate) -> bool {
    matches!(
        update,
        SessionUpdate::AgentMessageChunk(_)
            | SessionUpdate::AgentThoughtChunk(_)
            | SessionUpdate::ToolCall(_)
            | SessionUpdate::ToolCallUpdate(_)
    )
}

/// The one retry after a load is allowed only before output and only once.
fn stale_load_retry_allowed(
    reusable: bool,
    produced_output: bool,
    has_context: bool,
    restoration: LoadRestoration,
) -> bool {
    reusable && !produced_output && has_context && restoration == LoadRestoration::Restored
}

/// Reads the model the restored provider last reported, for the replacement handshake.
fn current_model(options: &[SessionConfigOption]) -> Option<String> {
    let option = options
        .iter()
        .find(|option| matches!(option.category, Some(SessionConfigOptionCategory::Model)))
        .or_else(|| {
            let selects: Vec<&SessionConfigOption> = options
                .iter()
                .filter(|option| matches!(option.kind, SessionConfigKind::Select(_)))
                .collect();
            (selects.len() == 1).then(|| selects[0])
        })?;
    let SessionConfigKind::Select(select) = &option.kind else {
        return None;
    };
    Some(select.current_value.0.to_string())
}

/// One attempt at a prompt, including the single rebuild retry after a stale `session/load`.
struct PromptAttempt {
    /// Blocks a rebuilt channel must receive; `None` builds the ordinary prompt.
    blocks: Option<Vec<ContentBlock>>,
    /// The prompt was already recorded before the first attempt, so a retry must not repeat it.
    skip_record: bool,
    /// Only the attempt after a load may rebuild; the retry attempt may not recurse again.
    allow_retry: bool,
}

impl PromptAttempt {
    fn initial() -> Self {
        Self {
            blocks: None,
            skip_record: false,
            allow_retry: true,
        }
    }
}

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Streams one prompt while routing only events that belong to this provider session.
    pub(super) async fn run_prompt(
        &mut self,
        operation_id: u64,
        prompt: Vec<ContentBlock>,
        record_prompt: RecordedTurn,
        inactivity_policy: PromptInactivityPolicy,
        end_report: PromptEndReport,
        events: mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
    ) {
        self.run_prompt_attempt(
            operation_id,
            prompt,
            record_prompt,
            inactivity_policy,
            end_report,
            events,
            PromptAttempt::initial(),
        )
        .await
    }

    /// Runs one prompt attempt; a stale restored session can rebuild and call this again once.
    #[allow(clippy::too_many_arguments)]
    async fn run_prompt_attempt(
        &mut self,
        operation_id: u64,
        prompt: Vec<ContentBlock>,
        record_prompt: RecordedTurn,
        inactivity_policy: PromptInactivityPolicy,
        end_report: PromptEndReport,
        events: mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
        mut attempt: PromptAttempt,
    ) {
        let original_prompt = prompt.clone();
        // An exit without a terminal provider response cannot prove remote work stopped.
        self.record_cleanup(operation_id, Err(runtime_unavailable()));
        let Some(mut channel) = self.channel.take() else {
            // Nothing was sent, so no side of the turn can be credited with ending it.
            end_report.record(PromptEndCause::Unknown);
            return;
        };
        let client = channel.connection.client.clone();
        // Catch events that arrived after the previous operation ended but before this command
        // was accepted. Setup updates in `pending_updates` are intentional and stay separate.
        drain_idle_events(&client, &mut channel.events, &self.command_sender).await;
        if let Ok(control) = channel.controls.try_recv() {
            match control {
                SessionControl::QueueOverflow => {
                    end_report.record(PromptEndCause::Unknown);
                    let _ = events.try_send(Err(session_event_overflow(
                        "session event queue overflowed",
                    )));
                    self.isolate_channel(channel).await;
                    return;
                }
                SessionControl::ConnectionLost(error) => {
                    end_report.record(PromptEndCause::SessionCrashed);
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
                    tool_timing: None,
                }))
                .is_err()
            {
                end_report.record(PromptEndCause::StreamDropped);
                self.isolate_channel(channel).await;
                return;
            }
        }
        let content_count = prompt.len();
        // Cheap cutoff only; the transcript is rendered if the prompt actually fails. It is taken
        // before the current prompt is recorded, so the rebuilt agent receives history up to this
        // turn and the prompt exactly once.
        let retry_cutoff = (attempt.allow_retry
            && self.load_restoration == LoadRestoration::Restored)
            .then(|| self.recorder.durable_bytes());
        let AgentPrompt {
            blocks,
            settles_handoff,
        } = match attempt.blocks.take() {
            Some(blocks) => AgentPrompt {
                blocks,
                // A rebuilt channel receives the transcript with this prompt, so accepting it
                // both settles the handoff and earns the new binding.
                settles_handoff: true,
            },
            None => prompt_for_agent(self, &prompt),
        };
        let recorded = record_prompt.blocks.as_deref().unwrap_or(&prompt);
        let stopped_recording = if attempt.skip_record {
            false
        } else {
            let outcome = match record_prompt.message_id.clone() {
                Some(message_id) => self.recorder.record_identified_prompt(recorded, message_id),
                None => self.recorder.record_prompt(recorded),
            };
            let stopped = matches!(outcome, RecordOutcome::JustFailed { .. });
            self.settle_record(outcome);
            stopped
        };
        if stopped_recording {
            // A turn already streaming is allowed to finish, because the agent's
            // work is real whether or not the file kept it. This one has not
            // started: nothing is lost by refusing it, and running it would put
            // the conversation somewhere the record cannot follow.
            end_report.record(PromptEndCause::Unknown);
            let _ = events.try_send(Err(history_degraded()));
            self.channel = Some(channel);
            return;
        }
        let agent_session_id = self.provider_session_id().to_string();
        let request = PromptRequest::new(agent_session_id.clone(), blocks);
        ora_debug!(session_id = %self.session.id, content_count = content_count, policy = ?inactivity_policy, "session/prompt sent");
        let mut pending = match client
            .start_session_request::<_, PromptResponse>(
                AcpSessionId::new(agent_session_id.clone()),
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
                end_report.record(PromptEndCause::Unknown);
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
            // It is also what earns a rebuilt binding its place in the row: the
            // provider session is only worth pointing at once it has been told what
            // the conversation is.
            let debt = std::mem::replace(&mut self.handoff, HandoffDebt::Settled);
            self.commit_rebuilt_binding();
            if let HandoffDebt::Recorded = debt {
                // Recording can itself fail, which degrades the session and leaves no
                // delivery line behind. The next actor then reads the binding as still
                // owing a handoff and sends it again, the harmless direction to be wrong in.
                let outcome = self.recorder.record_handoff_delivered(agent_session_id);
                self.settle_record(outcome);
            }
        }
        let mut permissions = HashMap::new();
        let mut followers = SessionFollowers::new();
        let mut input_state = ActiveInputState::default();
        let mut saw_output = false;
        #[cfg(feature = "test-support")]
        let windows = self
            .test_prompt_inactivity_windows
            .unwrap_or(&PROMPT_INACTIVITY_WINDOWS);
        #[cfg(not(feature = "test-support"))]
        let windows = &PROMPT_INACTIVITY_WINDOWS;
        let mut liveness = PromptLiveness::new(windows);
        let mut tool_timings = ToolTimings::default();
        loop {
            let input = tokio::select! {
                input = input_state.recv(
                    &mut channel.events,
                    &mut channel.controls,
                    &mut self.commands,
                ) => Some(input),
                () = liveness.wait(inactivity_policy) => None,
            };
            let Some(input) = input else {
                match retry_stalled_prompt(
                    self,
                    &mut channel,
                    pending,
                    &events,
                    &mut liveness,
                    &mut permissions,
                    &request,
                )
                .await
                {
                    StalledPrompt::Resent(resent) => {
                        pending = resent;
                        continue;
                    }
                    StalledPrompt::Failed(error) => {
                        // The inactivity schedule gave up: this is a system timeout, never the
                        // user's own stop, even though both end the turn as `Cancelled`.
                        end_report.record(PromptEndCause::InactivityTimeout);
                        self.end_timed_turn(StopReason::Cancelled, &tool_timings);
                        followers.finish(StopReason::Cancelled);
                        let _ = events.try_send(Err(error));
                        self.isolate_channel(channel).await;
                        return;
                    }
                }
            };
            match input {
                ActiveInput::Event(SessionEvent::Update(update)) => {
                    saw_output |= produced_output(&update.update);
                    // Record before forwarding: a client that drops mid-turn must not also cost
                    // the durable record of what the provider produced.
                    self.observe_session_update(&update.update);
                    let update = update.update;
                    liveness.observe(&update);
                    let tool_timing = tool_timings.observe(&update);
                    let outcome = match &tool_timing {
                        Some(timing) => self.recorder.record_timed_update(&update, timing),
                        None => self.recorder.record_update(&update),
                    };
                    self.settle_record(outcome);
                    followers.send_update(&update, tool_timing.clone());
                    if events
                        .try_send(Ok(PromptSessionEvent::SessionUpdate {
                            update,
                            tool_timing,
                        }))
                        .is_err()
                    {
                        // The turn's owner stopped reading, so nobody is left to answer it.
                        end_report.record(PromptEndCause::StreamDropped);
                        self.end_timed_turn(StopReason::Cancelled, &tool_timings);
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
                        end_report.record(PromptEndCause::Unknown);
                        self.end_timed_turn(StopReason::Cancelled, &tool_timings);
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
                        end_report.record(PromptEndCause::Unknown);
                        self.end_timed_turn(StopReason::Cancelled, &tool_timings);
                        followers.finish(StopReason::Cancelled);
                        self.cancel(&client, &permissions).await;
                        self.isolate_channel(channel).await;
                        return;
                    }
                    liveness.permission_settled(&permission.request.tool_call);
                }
                ActiveInput::Event(SessionEvent::Response(response)) => {
                    if !pending.matches_response(&response) {
                        continue;
                    }
                    match pending.finish(response) {
                        Ok(response) => {
                            self.record_cleanup(operation_id, Ok(()));
                            // The provider answered: its stop reason now describes the turn, so the
                            // end cause is not an interruption.
                            end_report.record(PromptEndCause::Settled);
                            ora_debug!(session_id = %self.session.id, stop_reason = ?response.stop_reason, "prompt completed");
                            let token_usage = usage::normalize_token_usage(
                                &self.session.agent_ref,
                                response.usage.as_ref(),
                                response.meta.as_ref(),
                                &usage::NoUsageExtensions,
                            );
                            self.end_timed_turn(response.stop_reason, &tool_timings);
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
                                self.isolate_channel(channel).await;
                            }
                        }
                        Err(error) => {
                            let reusable = matches!(&error, ora_acp::AcpError::RequestFailed(_));
                            // A load can claim success while the provider session is gone; its
                            // first prompt then fails before producing anything. Rebuild once and
                            // resend the same prompt with the transcript this side still owns.
                            let retry_blocks = retry_cutoff
                                .and_then(|cutoff| rebuild_prompt(self, &original_prompt, cutoff));
                            let retry_stale_load = stale_load_retry_allowed(
                                reusable,
                                saw_output,
                                retry_blocks.is_some(),
                                self.load_restoration,
                            );
                            if retry_stale_load {
                                ora_debug!(
                                    session_id = %self.session.id,
                                    "first prompt after session/load failed without output; rebuilding and retrying",
                                );
                                followers.finish(StopReason::Cancelled);
                                let model = current_model(&self.reported_config_options);
                                self.isolate_channel(channel).await;
                                match self.rebuild_provider_session(model.as_deref()).await {
                                    Ok(setup) => {
                                        if !super::actor::publish_setup(&events, setup) {
                                            self.record_cleanup(operation_id, Ok(()));
                                            end_report.record(PromptEndCause::StreamDropped);
                                            self.end_timed_turn(
                                                StopReason::Cancelled,
                                                &tool_timings,
                                            );
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
                                        self.end_timed_turn(StopReason::Cancelled, &tool_timings);
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
                            self.end_timed_turn(StopReason::Cancelled, &tool_timings);
                            followers.finish(StopReason::Cancelled);
                            self.load_restoration = LoadRestoration::NotRestored;
                            let delivered = events.try_send(Err(map_prompt_error(error))).is_ok();
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
                    end_report.record(PromptEndCause::SessionCrashed);
                    self.end_timed_turn(StopReason::Cancelled, &tool_timings);
                    followers.finish(StopReason::Cancelled);
                    self.fail_prompt(&events, error);
                    return;
                }
                ActiveInput::Control(SessionControl::QueueOverflow) => {
                    end_report.record(PromptEndCause::Unknown);
                    self.end_timed_turn(StopReason::Cancelled, &tool_timings);
                    followers.finish(StopReason::Cancelled);
                    self.cancel(&client, &permissions).await;
                    let _ = events.try_send(Err(session_event_overflow(
                        "session event queue overflowed",
                    )));
                    self.isolate_channel(channel).await;
                    return;
                }
                ActiveInput::EventsClosed | ActiveInput::ControlsClosed => {
                    end_report.record(PromptEndCause::SessionCrashed);
                    self.end_timed_turn(StopReason::Cancelled, &tool_timings);
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
                    completion,
                }) if followers.contains(cancelled) => {
                    // Waiting for a view's disk replay must not stall the independent prompt.
                    let cleanup = followers.remove_and_wait(cancelled);
                    tokio::spawn(async move {
                        let result = cleanup.await;
                        if let Some(completion) = completion {
                            let _ = completion.send(result);
                        }
                    });
                }
                ActiveInput::Command(command)
                    if matches!(&command, RuntimeCommand::CancelActivePrompt)
                        || matches!(
                            &command,
                            RuntimeCommand::Cancel {
                                operation_id: cancelled, ..
                            } if *cancelled == operation_id
                        ) =>
                {
                    let notify_owner = matches!(command, RuntimeCommand::CancelActivePrompt);
                    let completion = match command {
                        RuntimeCommand::Cancel { completion, .. } => completion,
                        _ => None,
                    };
                    // `cancel_prompt` is the user pressing stop, while the owner-addressed cancel
                    // is a consumer that stopped reading; the two must not be reported alike.
                    end_report.record(if notify_owner {
                        PromptEndCause::UserStopped
                    } else {
                        PromptEndCause::StreamDropped
                    });
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
                    self.end_timed_turn(StopReason::Cancelled, &tool_timings);
                    followers.finish(StopReason::Cancelled);
                    let owner_notified = !notify_owner
                        || events
                            .try_send(Ok(PromptSessionEvent::Completed {
                                stop_reason: StopReason::Cancelled,
                                token_usage: None,
                            }))
                            .is_ok();
                    if reusable && owner_notified {
                        self.channel = Some(channel);
                    } else {
                        self.isolate_channel(channel).await;
                    }
                    let result = if reusable {
                        Ok(())
                    } else {
                        Err(RuntimeError::internal(
                            "provider did not confirm cancelled operation stopped",
                            std::io::Error::other("cancellation grace expired or provider failed"),
                        ))
                    };
                    self.record_cleanup(operation_id, result.clone());
                    if let Some(completion) = completion {
                        let _ = completion.send(result);
                    }
                    return;
                }
                ActiveInput::Command(RuntimeCommand::Stop { response }) => {
                    end_report.record(PromptEndCause::Unknown);
                    self.cancel(&client, &permissions).await;
                    self.end_timed_turn(StopReason::Cancelled, &tool_timings);
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
                    cleanup,
                    operation_id,
                    events,
                    accepted,
                }) => {
                    // Capture the durable cutoff, the in-progress pending records, and the live
                    // follower registration atomically (no await), then let the follower's relay
                    // task stream the merged prefix before live events. The actor returns to its
                    // select loop immediately, so a slow view can never backpressure the prompt.
                    self.record_cleanup(operation_id, Ok(()));
                    let cutoff = self.recorder.durable_bytes();
                    let pending = self.recorder.pending_records();
                    if accepted.send(Ok(())).is_ok() {
                        followers.insert(
                            operation_id,
                            FollowerOutput {
                                events,
                                completion: cleanup,
                            },
                            self.sessions_root.clone(),
                            self.session.id.to_string(),
                            cutoff,
                            pending,
                        );
                    }
                }
                ActiveInput::Command(RuntimeCommand::ClaimDirectProviderCall { response }) => {
                    let _ = response.send(self.provider_session_id().to_string());
                }
                ActiveInput::Command(RuntimeCommand::AdoptUserTitle { title, response }) => {
                    self.adopt_user_title(title);
                    let _ = response.send(());
                }
                ActiveInput::Command(RuntimeCommand::Cancel {
                    operation_id,
                    completion,
                }) => {
                    if let Some(completion) = completion {
                        let _ = completion.send(self.cleanup_outcome(operation_id));
                    }
                }
                ActiveInput::Command(RuntimeCommand::CancelActivePrompt) => {}
                ActiveInput::Command(RuntimeCommand::McpDesiredMaybeChanged) => {
                    self.note_desired_mcp();
                }
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
                    end_report.record(PromptEndCause::SessionCrashed);
                    self.cancel(&client, &permissions).await;
                    self.end_timed_turn(StopReason::Cancelled, &tool_timings);
                    followers.finish(StopReason::Cancelled);
                    self.isolate_channel(channel).await;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{current_model, produced_output, stale_load_retry_allowed};
    use crate::attach::LoadRestoration;
    use agent_client_protocol_schema::v1::{
        ContentBlock, ContentChunk, Plan, SessionUpdate, TextContent, ToolCall, ToolCallUpdate,
        ToolCallUpdateFields,
    };
    use agent_client_protocol_schema::v1::{
        SessionConfigId, SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
        SessionConfigSelect, SessionConfigSelectOption, SessionConfigSelectOptions,
        SessionConfigValueId,
    };

    #[test]
    fn a_stale_load_retry_requires_no_output_and_only_one_attempt() {
        assert!(stale_load_retry_allowed(
            true,
            false,
            true,
            LoadRestoration::Restored
        ));
        assert!(!stale_load_retry_allowed(
            true,
            true,
            true,
            LoadRestoration::Restored
        ));
        assert!(!stale_load_retry_allowed(
            true,
            false,
            false,
            LoadRestoration::Restored
        ));
        assert!(!stale_load_retry_allowed(
            false,
            false,
            true,
            LoadRestoration::Restored
        ));
    }

    #[test]
    fn output_classification_covers_text_tools_and_controls() {
        let text = || {
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new("hello"),
            )))
        };
        for update in [
            text(),
            SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new("thinking"),
            ))),
            SessionUpdate::ToolCall(ToolCall::new("tool", "Tool")),
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("tool", ToolCallUpdateFields::new())),
        ] {
            assert!(produced_output(&update), "{update:?}");
        }
        // A plan can fail without any assistant output or tool side effect and is retryable.
        assert!(!produced_output(&SessionUpdate::Plan(
            Plan::new(Vec::new())
        )));
    }

    #[test]
    fn an_empty_provider_report_has_no_rebuild_model() {
        assert_eq!(current_model(&[]), None);
    }

    #[test]
    fn the_rebuild_model_comes_from_the_restored_provider() {
        let option = SessionConfigOption::new(
            SessionConfigId::new("model"),
            "Model",
            SessionConfigKind::Select(SessionConfigSelect::new(
                SessionConfigValueId::new("deepseek/deepseek-v4-pro"),
                SessionConfigSelectOptions::Ungrouped(vec![SessionConfigSelectOption::new(
                    "deepseek/deepseek-v4-pro",
                    "DeepSeek V4 Pro",
                )]),
            )),
        )
        .category(SessionConfigOptionCategory::Model);
        assert_eq!(
            current_model(&[option]),
            Some("deepseek/deepseek-v4-pro".to_string())
        );
    }
}
