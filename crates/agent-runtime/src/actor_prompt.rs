//! One prompt turn: admission, provider streaming, and the end it reports.
//!
//! Kept apart from the actor's command loop so the turn's many termination paths stay readable
//! together: each one records why the turn ended before its owner can observe it settled.

use super::handoff::{AgentPrompt, prompt_for_agent, rebuild_prompt};
use super::prompt_liveness::PromptLiveness;
use super::prompt_retry::{StalledPrompt, retry_stalled_prompt};
use super::scheduling::{ActiveInput, ActiveInputState};
use super::session_followers::{FollowerOutput, SessionFollowers};
use super::title_acquisition::PollAttempt;
use super::tool_timing::ToolTimings;
use super::*;
use crate::attach::LoadRestoration;
use crate::host::SkillCatalogSource;
use agent_client_protocol_schema::v1::SessionId as AcpSessionId;
use agent_client_protocol_schema::v1::{
    AGENT_METHOD_NAMES, PromptRequest, PromptResponse, StopReason,
};
use ora_history::TurnEndedBy;
use ora_logging::{ora_debug, ora_warn};
use stale_load::{describes_provider, stale_load_retry_allowed};

/// Whether one provider update is work the user would see or a tool already ran.
///
/// `Plan` is liveness activity, not output: it carries no assistant text and no tool side effect,
/// so a failed prompt that only planned may still be retried safely.
pub(crate) fn produced_output(update: &SessionUpdate) -> bool {
    matches!(
        update,
        SessionUpdate::AgentMessageChunk(_)
            | SessionUpdate::AgentThoughtChunk(_)
            | SessionUpdate::ToolCall(_)
            | SessionUpdate::ToolCallUpdate(_)
    )
}

/// One attempt at a prompt, including the single rebuild retry after a stale `session/load`.
struct PromptAttempt {
    /// Prompt prepared for a rebuilt channel; `None` builds the ordinary prompt.
    blocks: Option<AgentPrompt>,
    /// The prompt was already recorded before the first attempt, so a retry must not repeat it.
    skip_record: bool,
    /// Only the attempt after a load may rebuild; the retry attempt may not recurse again.
    allow_retry: bool,
    /// Followers collected so far; the retry continues the same logical turn.
    followers: Option<SessionFollowers>,
}

impl PromptAttempt {
    fn initial() -> Self {
        Self {
            blocks: None,
            skip_record: false,
            allow_retry: true,
            followers: None,
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
        // Followers belong to the logical turn, so take them before any early return can drop
        // the attempt without a terminal boundary.
        let mut followers = attempt
            .followers
            .take()
            .unwrap_or_else(SessionFollowers::new);
        // An exit without a terminal provider response cannot prove remote work stopped.
        self.record_cleanup(operation_id, Err(runtime_unavailable()));
        let Some(mut channel) = self.channel.take() else {
            // Nothing was sent, so no side of the turn can be credited with ending it.
            end_report.record(PromptEndCause::Unknown);
            followers.finish(StopReason::Cancelled);
            return;
        };
        let client = channel.connection.client.clone();
        // Catch events that arrived after the previous operation ended but before this command
        // was accepted. Setup updates in `pending_updates` are intentional and stay separate.
        drain_idle_events(self, &mut channel).await;
        if let Ok(control) = channel.controls.try_recv() {
            match control {
                SessionControl::QueueOverflow => {
                    end_report.record(PromptEndCause::Unknown);
                    let _ = events.try_send(Err(session_event_overflow(
                        "session event queue overflowed",
                    )));
                    self.isolate_channel(channel).await;
                    if attempt.skip_record {
                        self.end_turn(StopReason::Cancelled);
                    }
                    followers.finish(StopReason::Cancelled);
                    return;
                }
                SessionControl::ConnectionLost(error) => {
                    end_report.record(PromptEndCause::SessionCrashed);
                    self.fail_prompt(&events, error);
                    if attempt.skip_record {
                        self.end_turn(StopReason::Cancelled);
                    }
                    followers.finish(StopReason::Cancelled);
                    return;
                }
            }
        }
        while let Some(notification) = channel.pending_updates.pop_front() {
            let update = match notification.update {
                // Never taken again: the stream already started from the options the session is on.
                SessionUpdate::ConfigOptionUpdate(_) => continue,
                update => {
                    self.observe_session_update(&update);
                    update
                }
            };
            // Only a retry already has followers here, and they joined on the session it replaced.
            if describes_provider(&update) {
                followers.send_update(&update, None);
            }
            if events
                .try_send(Ok(PromptSessionEvent::SessionUpdate {
                    update,
                    tool_timing: None,
                }))
                .is_err()
            {
                end_report.record(PromptEndCause::StreamDropped);
                self.end_on_channel(channel, /*usable*/ true, /*told*/ false)
                    .await;
                if attempt.skip_record {
                    self.end_turn(StopReason::Cancelled);
                }
                followers.finish(StopReason::Cancelled);
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
            prefix_len,
        } = match attempt.blocks.take() {
            Some(prompt) => prompt,
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
            followers.finish(StopReason::Cancelled);
            return;
        }
        if self.turn_cancel.deadline().is_some() && !attempt.skip_record {
            self.finish_unsent_prompt(channel, &end_report, &events, &mut followers);
            return;
        }
        let (blocks, catalog_commit) = match self
            .skill_catalog
            .skill_catalog(&self.session.workspace_id)
        {
            Ok(current_skills) => {
                let skill_injection = crate::skill_catalog::prepare_skill_catalog_injection(
                    self.announced_skills.as_ref(),
                    &current_skills,
                );
                let blocks = match &skill_injection {
                    Some(injection) => crate::skill_catalog::insert_catalog_block(
                        blocks,
                        injection.block_text.clone(),
                        prefix_len,
                    ),
                    None => blocks,
                };
                (blocks, Some(skill_injection))
            }
            Err(error) => {
                ora_warn!(
                    session_id = %self.session.id,
                    workspace_id = %self.session.workspace_id,
                    error = %error,
                    "failed to read skill catalog for prompt; skipping catalog injection for this turn",
                );
                (blocks, None)
            }
        };
        let agent_session_id = self.provider_session_id().to_string();
        let mut request = PromptRequest::new(agent_session_id.clone(), blocks);
        if let Some(instruction) = &record_prompt.attempt_instruction {
            request.prompt.push(instruction());
        }
        ora_debug!(session_id = %self.session.id, content_count = content_count, policy = ?inactivity_policy, "session/prompt sent");
        let mut pending = match client
            .start_session_request::<_, PromptResponse>(
                AcpSessionId::new(agent_session_id.clone()),
                AGENT_METHOD_NAMES.session_prompt,
                &request,
            )
            .await
        {
            Ok(pending) => {
                // Commit announced skill paths only once the ACP request is accepted on the wire:
                // any failure before admission or send leaves the next prompt free to re-emit.
                // A catalog read error skips both injection and commit so the next turn retries
                // against the previous announced baseline. An initial turn with a readable empty
                // catalog records the empty announced set (ADR D3) so a skill installed later
                // arrives as an incremental update rather than full.
                if let Some(skill_injection) = catalog_commit {
                    if let Some(injection) = skill_injection {
                        self.announced_skills = Some(injection.next_announced);
                    } else {
                        self.announced_skills.get_or_insert_with(Default::default);
                    }
                }
                pending
            }
            Err(error) => {
                // The request never reached the agent, so a transcript this prompt
                // was carrying is still owed and stays owed — in memory and, since
                // no delivery was recorded, across a restart as well.
                end_report.record(PromptEndCause::Unknown);
                self.end_turn(StopReason::Cancelled);
                let _ = events.try_send(Err(map_acp_error(error)));
                self.isolate_channel(channel).await;
                followers.finish(StopReason::Cancelled);
                return;
            }
        };
        #[cfg(feature = "test-support")]
        let windows = self
            .test_prompt_inactivity_windows
            .unwrap_or(&PROMPT_INACTIVITY_WINDOWS);
        #[cfg(not(feature = "test-support"))]
        let windows = &PROMPT_INACTIVITY_WINDOWS;
        let mut liveness = PromptLiveness::for_policy(windows, inactivity_policy);
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
        let mut permissions = crate::prompt_permissions::PromptPermissions::default();
        let mut input_state = ActiveInputState::default();
        let mut saw_output = false;
        let mut tool_timings = ToolTimings::default();
        loop {
            // At expiry, drain only the inputs already ready at the boundary. A flowing
            // queue of unrelated notifications must not postpone a timeout indefinitely.
            let input = if let Some(deadline) = liveness.wait_deadline() {
                let boundary = input_state.boundary_deadline().unwrap_or(deadline);
                if tokio::time::Instant::now() >= boundary {
                    let next = input_state.at_wait_deadline(
                        boundary,
                        &mut channel.events,
                        &mut channel.controls,
                        &mut self.commands,
                        &self.turn_cancel,
                    );
                    if next.is_none() && boundary != deadline {
                        input_state.clear_wait_boundary();
                        continue;
                    }
                    next
                } else if self.turn_cancel.deadline().is_none()
                    && let Some(permission) = permissions.take_deferred()
                {
                    Some(ActiveInput::Event(SessionEvent::Permission(permission)))
                } else {
                    tokio::select! {
                        biased;
                        () = tokio::time::sleep_until(deadline) => continue,
                        input = input_state.recv(&mut channel.events, &mut channel.controls, &mut self.commands) => Some(input),
                    }
                }
            } else {
                tokio::select! {
                    input = input_state.recv(&mut channel.events, &mut channel.controls, &mut self.commands) => Some(input),
                    () = liveness.wait(), if inactivity_policy == PromptInactivityPolicy::Timeout => None,
                }
            };
            let Some(input) = input else {
                if liveness.wait_deadline().is_some() {
                    self.finish_expired_wait(
                        channel,
                        pending,
                        liveness.snapshot(),
                        crate::turn_cancel::PromptStop {
                            operation_id,
                            client: &client,
                            permissions: &permissions.pending,
                            events: &events,
                            end_report: &end_report,
                            followers: &mut followers,
                            tool_timings: &tool_timings,
                            saw_output: &mut saw_output,
                        },
                    )
                    .await;
                    return;
                }
                match retry_stalled_prompt(
                    self,
                    &mut channel,
                    pending,
                    &events,
                    &mut liveness,
                    &mut permissions.pending,
                    &mut request,
                    record_prompt.attempt_instruction.as_deref(),
                    &mut saw_output,
                    &mut followers,
                )
                .await
                {
                    StalledPrompt::Resent(resent) => {
                        tool_timings = ToolTimings::default();
                        pending = resent;
                        continue;
                    }
                    StalledPrompt::Cancelled {
                        reusable,
                        session_invalid,
                    } => {
                        // A turn cancel merged into the watchdog's stop: the turn ends as the
                        // user's cancel, never resent, and only an answered cancel keeps the
                        // provider session.
                        end_report.record(PromptEndCause::UserStopped);
                        end_report.mark_cancel_applied();
                        self.end_timed_turn_with(
                            StopReason::Cancelled,
                            &tool_timings,
                            Some(TurnEndedBy::CallerCancel),
                            None,
                        );
                        followers.finish(StopReason::Cancelled);
                        if reusable {
                            self.record_cleanup(operation_id, Ok(()));
                            let told = events
                                .try_send(Ok(PromptSessionEvent::Completed {
                                    stop_reason: StopReason::Cancelled,
                                    token_usage: None,
                                }))
                                .is_ok();
                            self.end_on_channel(channel, !session_invalid, told).await;
                        } else {
                            let _ = events.try_send(Err(RuntimeError::turn_cancel_unanswered()));
                            self.isolate_channel(channel).await;
                        }
                        return;
                    }
                    StalledPrompt::Failed(error) => {
                        // The inactivity schedule gave up: this is a system timeout, never the
                        // user's own stop, even though both end the turn as `Cancelled`.
                        end_report.record(PromptEndCause::InactivityTimeout);
                        let turn_failure = error.turn_failure().cloned();
                        self.end_timed_turn_with(
                            StopReason::Cancelled,
                            &tool_timings,
                            None,
                            turn_failure,
                        );
                        followers.finish(StopReason::Cancelled);
                        let _ = events.try_send(Err(error));
                        self.isolate_channel(channel).await;
                        return;
                    }
                }
            };
            match input {
                ActiveInput::Event(SessionEvent::Update(update, arrived)) => {
                    saw_output |= produced_output(&update.update);
                    // Record before forwarding: a client that drops mid-turn must not also cost
                    // the durable record of what the provider produced.
                    channel.observe_commands(&update.update);
                    self.observe_session_update(&update.update);
                    let update = update.update;
                    liveness.observe_at(
                        &update,
                        if liveness.wait_deadline().is_some() {
                            arrived
                        } else {
                            tokio::time::Instant::now()
                        },
                    );
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
                        self.accept_changes_on(&mut channel);
                        self.cancel(&client, &permissions.pending).await;
                        self.isolate_channel(channel).await;
                        return;
                    }
                }
                ActiveInput::Event(SessionEvent::Permission(permission)) => {
                    if let Err(error) = permissions
                        .answer(
                            &client,
                            &self.session.id,
                            permission,
                            &mut liveness,
                            input_state.boundary_deadline(),
                        )
                        .await
                    {
                        ora_warn!(session_id = %self.session.id, error = %error, "failed to answer permission request");
                        end_report.record(PromptEndCause::Unknown);
                        self.end_timed_turn(StopReason::Cancelled, &tool_timings);
                        followers.finish(StopReason::Cancelled);
                        self.cancel(&client, &permissions.pending).await;
                        self.isolate_channel(channel).await;
                        return;
                    }
                }
                ActiveInput::Event(SessionEvent::Response(response)) => {
                    if !pending.matches_response(&response) {
                        if let Some(update) = self.settle_config_change(&mut channel, response) {
                            followers.send_update(&update, None);
                            let _ = events.try_send(Ok(PromptSessionEvent::SessionUpdate {
                                update,
                                tool_timing: None,
                            }));
                        }
                        continue;
                    }
                    if !permissions.pending.is_empty() {
                        // The prompt answered during arbitration. Release its deferred requests before reusing the channel.
                        let _ = tokio::time::timeout(
                            CANCELLATION_GRACE,
                            crate::prompt_permissions::cancel_pending(
                                &client,
                                &permissions.pending,
                            ),
                        )
                        .await;
                    }
                    match pending.finish(response) {
                        Ok(response) => {
                            self.finish_completed_prompt(
                                channel,
                                response,
                                operation_id,
                                crate::turn_cancel::PromptEnd {
                                    events: &events,
                                    end_report: &end_report,
                                    followers: &mut followers,
                                    tool_timings: &tool_timings,
                                },
                            )
                            .await;
                        }
                        Err(error) => {
                            let reusable = matches!(&error, ora_acp::AcpError::RequestFailed(_));
                            let session_invalid = super::support::provider_session_invalid(&error);
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
                            if retry_stale_load && !self.session_mcp.preserve_provider_identity() {
                                ora_debug!(
                                    session_id = %self.session.id,
                                    "first prompt after session/load failed without output; rebuilding and retrying",
                                );
                                match self.replace_stale_session(channel).await {
                                    Ok(setup) => {
                                        for update in &setup {
                                            followers.send_update(update, None);
                                        }
                                        if !super::actor::publish_setup(&events, setup) {
                                            self.record_cleanup(operation_id, Ok(()));
                                            end_report.record(PromptEndCause::StreamDropped);
                                            self.end_timed_turn(
                                                StopReason::Cancelled,
                                                &tool_timings,
                                            );
                                            followers.finish(StopReason::Cancelled);
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
                                                followers: Some(followers),
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
                            let failure = map_prompt_error(error);
                            // A request error is the Agent's own answer; a transport failure is not.
                            let failure = if reusable {
                                failure.answered()
                            } else {
                                failure
                            };
                            let turn_failure = failure.turn_failure().cloned();
                            self.end_timed_turn_with(
                                StopReason::Cancelled,
                                &tool_timings,
                                None,
                                turn_failure,
                            );
                            followers.finish(StopReason::Cancelled);
                            self.load_restoration = LoadRestoration::NotRestored;
                            let delivered = events.try_send(Err(failure)).is_ok();
                            self.end_on_channel(channel, reusable && !session_invalid, delivered)
                                .await;
                        }
                    }
                    return;
                }
                // A lost connection both reports itself and closes the queue, and either may be read
                // first, so both end the turn the same way.
                lost @ (ActiveInput::Control(SessionControl::ConnectionLost(_))
                | ActiveInput::EventsClosed) => {
                    let error = match lost {
                        ActiveInput::Control(SessionControl::ConnectionLost(error)) => error,
                        _ => runtime_unavailable(),
                    };
                    end_report.record(PromptEndCause::SessionCrashed);
                    // The Agent died or the transport broke before any cancel was written: the
                    // turn fails as `agent_unavailable` (D2/D7).
                    self.end_timed_turn_with(
                        StopReason::Cancelled,
                        &tool_timings,
                        None,
                        Some(crate::turn_failure::agent_unavailable_failure()),
                    );
                    followers.finish(StopReason::Cancelled);
                    self.fail_prompt(&events, error);
                    return;
                }
                ActiveInput::Control(SessionControl::QueueOverflow) => {
                    end_report.record(PromptEndCause::Unknown);
                    // An Ora-side queue overflow is an internal failure, not the Agent's (D2).
                    self.end_timed_turn_with(
                        StopReason::Cancelled,
                        &tool_timings,
                        None,
                        Some(crate::turn_failure::internal_failure()),
                    );
                    followers.finish(StopReason::Cancelled);
                    self.cancel(&client, &permissions.pending).await;
                    let _ = events.try_send(Err(session_event_overflow(
                        "session event queue overflowed",
                    )));
                    self.isolate_channel(channel).await;
                    return;
                }
                ActiveInput::ControlsClosed => {
                    end_report.record(PromptEndCause::SessionCrashed);
                    // The host stopped controlling this turn, which only happens when the session
                    // is ending: the turn is attributed to the session end (D1).
                    self.end_timed_turn_with(
                        StopReason::Cancelled,
                        &tool_timings,
                        Some(TurnEndedBy::SessionEnd),
                        None,
                    );
                    followers.finish(StopReason::Cancelled);
                    self.fail_prompt(&events, runtime_unavailable());
                    return;
                }
                ActiveInput::Command(RuntimeCommand::RespondToPermission { request, response }) => {
                    let result =
                        respond_permission(&client, request, &mut permissions.pending).await;
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
                    self.settle_prompt_cancel(
                        channel,
                        pending,
                        crate::turn_cancel::PromptStop {
                            operation_id,
                            client: &client,
                            permissions: &permissions.pending,
                            events: &events,
                            end_report: &end_report,
                            followers: &mut followers,
                            tool_timings: &tool_timings,
                            saw_output: &mut saw_output,
                        },
                        notify_owner,
                        completion,
                    )
                    .await;
                    return;
                }
                ActiveInput::Command(RuntimeCommand::Stop { response }) => {
                    end_report.record(PromptEndCause::Unknown);
                    self.accept_changes_on(&mut channel);
                    self.cancel(&client, &permissions.pending).await;
                    // The host is ending the session (an `EndSession` or a D3 end decision), so
                    // this running turn is attributed to the session end (D1).
                    self.end_timed_turn_with(
                        StopReason::Cancelled,
                        &tool_timings,
                        Some(TurnEndedBy::SessionEnd),
                        None,
                    );
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
                    self.follow_prompt(
                        operation_id,
                        &channel,
                        &mut followers,
                        FollowerOutput {
                            events,
                            completion: cleanup,
                        },
                        accepted,
                    );
                }
                ActiveInput::Command(RuntimeCommand::ChangeConfig(request)) => {
                    self.change_config(Some(&mut channel), request).await;
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
                // A running prompt is not quiet; the question is only asked between turns.
                ActiveInput::Command(RuntimeCommand::TakeIdleTurnTraffic { response }) => {
                    let _ = response.send(true);
                }
                // The channel is borrowed, not gone: the session still has its provider.
                ActiveInput::Command(RuntimeCommand::ReadOptions { response }) => {
                    let _ = response.send(Some(self.reported_config_options.clone()));
                }
                ActiveInput::Command(RuntimeCommand::McpDesiredMaybeChanged) => {
                    self.note_desired_mcp();
                }
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
                    self.cancel(&client, &permissions.pending).await;
                    self.end_timed_turn(StopReason::Cancelled, &tool_timings);
                    followers.finish(StopReason::Cancelled);
                    self.isolate_channel(channel).await;
                    return;
                }
            }
        }
    }
}

#[path = "actor_prompt/completion.rs"]
mod completion;
#[path = "actor_prompt/follow.rs"]
mod follow;
#[path = "actor_prompt/stale_load.rs"]
mod stale_load;
#[cfg(test)]
#[path = "actor_prompt/tests.rs"]
mod tests;
