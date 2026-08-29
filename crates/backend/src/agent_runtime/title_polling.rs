use super::super::RuntimeCommand;
use super::super::events::settle_idle_event;
use super::super::routing::{SessionControl, SessionEvent};
use super::super::support::contract_session;
use super::super::title_acquisition::PollAttempt;
use super::RuntimeActor;
use super::permission_not_pending;
use agent_client_protocol_schema::v1::AGENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::StopReason;
use agent_client_protocol_schema::v1::{ListSessionsRequest, ListSessionsResponse, SessionUpdate};
use ora_application::{Clock, SessionRepository};
use ora_contracts::AppEvent;
use ora_contracts::StopSessionResponse;
use ora_domain::SessionTitle;
use ora_logging::{ora_debug, ora_warn};
use std::time::Duration;
use tokio::time::timeout;

impl RuntimeActor {
    /// Polls the provider's bounded session list without delaying user operations.
    pub(super) async fn run_title_poll(&mut self, attempt: PollAttempt) {
        if !self.title_acquisition.accepts_title() {
            return;
        }
        if !self.title_acquisition.should_send_list(attempt) {
            self.title_acquisition.finish_attempt(attempt);
            if matches!(attempt, PollAttempt::Final) {
                self.title_acquisition.close();
            }
            return;
        }

        let Some(mut channel) = self.channel.take() else {
            self.title_acquisition.close();
            self.mark_stopped();
            return;
        };
        let client = channel.connection.client.clone();
        let request = ListSessionsRequest::new().cwd(self.cwd.clone());
        let request = timeout(
            Duration::from_secs(5),
            client.request::<_, ListSessionsResponse>(AGENT_METHOD_NAMES.session_list, &request),
        );
        tokio::pin!(request);
        let mut final_attempt_pending = false;

        loop {
            // A ready user command must win even when the provider response becomes ready in the
            // same poll. Keeping the request below commands and terminal controls also prevents
            // its first write from starting when user work was already waiting in the actor queue.
            tokio::select! {
                biased;
                command = self.commands.recv() => {
                    let Some(command) = command else {
                        self.title_acquisition.close();
                        self.release().await;
                        return;
                    };
                    match command {
                        RuntimeCommand::Load { operation_id, events, accepted } => {
                            self.channel = Some(channel);
                            self.title_acquisition.preempt_attempt(attempt);
                            // run_load resolves `accepted` only after the Running
                            // row is persisted (see actor.rs for the ordering).
                            self.run_load(operation_id, events, accepted).await;
                            return;
                        }
                        RuntimeCommand::Prompt { operation_id, prompt, record_prompt, events, accepted } => {
                            self.channel = Some(channel);
                            self.title_acquisition.preempt_attempt(attempt);
                            let _ = accepted.send(Ok(()));
                            self.run_prompt(operation_id, prompt, record_prompt, events).await;
                            return;
                        }
                        RuntimeCommand::Stop { response } => {
                            self.channel = Some(channel);
                            self.title_acquisition.close();
                            self.unload().await;
                            let _ = response.send(Ok(StopSessionResponse {
                                session: contract_session(self.session.clone()),
                            }));
                            return;
                        }
                        RuntimeCommand::RespondToPermission { response, .. } => {
                            self.channel = Some(channel);
                            self.title_acquisition.preempt_attempt(attempt);
                            let _ = response.send(Err(permission_not_pending()));
                            return;
                        }
                        RuntimeCommand::AgentProcessReplaced { agent } => {
                            // The channel goes back first: the detach decision is about the live
                            // registration this poll borrowed, and it cannot see one that is still
                            // moved out. Ending the attempt either way costs at most a title.
                            self.channel = Some(channel);
                            self.title_acquisition.preempt_attempt(attempt);
                            self.detach_replaced_agent(&agent);
                            return;
                        }
                        RuntimeCommand::Cancel { .. } => {
                            // A prompt stream sends this after its Completed event is consumed;
                            // it is not a cancellation of the independent title fallback.
                        }
                        RuntimeCommand::CancelActivePrompt => {}
                        RuntimeCommand::PreemptTitlePolling { response } => {
                            self.channel = Some(channel);
                            self.title_acquisition.preempt_attempt(attempt);
                            let _ = response.send(());
                            return;
                        }
                        RuntimeCommand::AdoptUserTitle { title, response } => {
                            self.channel = Some(channel);
                            self.adopt_user_title(title);
                            let _ = response.send(());
                            return;
                        }
                        RuntimeCommand::TitleUpdate { update } => {
                            self.observe_session_update(&update);
                        }
                        RuntimeCommand::TitlePoll {
                            attempt: PollAttempt::Final,
                        } if matches!(attempt, PollAttempt::First) => {
                            // The final timer may have fired while the first request was still
                            // active. Defer it until the current request restores the channel so
                            // the final fallback cannot be lost to an internal command.
                            final_attempt_pending = true;
                        }
                        RuntimeCommand::TitlePoll { .. } => {
                            // Duplicate or stale scheduler commands have no work while this
                            // attempt is already in flight.
                        }
                    }
                }
                control = channel.controls.recv() => {
                    match control {
                        Some(SessionControl::ConnectionLost(_)) | None => {
                            self.title_acquisition.close();
                            self.mark_stopped();
                            return;
                        }
                        Some(SessionControl::QueueOverflow) => {
                            self.title_acquisition.close();
                            self.isolate_channel(channel).await;
                            return;
                        }
                    }
                }
                result = &mut request => {
                    self.title_acquisition.finish_attempt(attempt);
                    match result {
                        Ok(Ok(response)) => {
                            if let Some(title) = response
                                .sessions
                                .into_iter()
                                .find(|session| {
                                    session.session_id.0.as_ref() == self.session.agent_session_id.as_str()
                                })
                                .and_then(|session| session.title)
                            {
                                self.persist_agent_title(&title);
                            }
                        }
                        Ok(Err(error)) => {
                            ora_debug!(
                                session_id = %self.session.id,
                                error = %error,
                                attempt = ?attempt,
                                "session/list title polling failed",
                            );
                        }
                        Err(_) => {
                            ora_debug!(
                                session_id = %self.session.id,
                                attempt = ?attempt,
                                "session/list title polling timed out",
                            );
                        }
                    }
                    if matches!(attempt, PollAttempt::Final) {
                        self.title_acquisition.close();
                    }
                    self.channel = Some(channel);
                    if final_attempt_pending
                        && let Some(command_sender) = self.command_sender.upgrade()
                    {
                        let _ = command_sender.send(RuntimeCommand::TitlePoll {
                            attempt: PollAttempt::Final,
                        });
                    }
                    return;
                }
                event = channel.events.recv() => {
                    match event {
                        Some(SessionEvent::Update(update)) => {
                            self.observe_session_update(&update.update);
                            channel.pending_updates.push_back(update);
                        }
                        Some(event) => {
                            settle_idle_event(&client, event).await;
                        }
                        None => {
                            self.title_acquisition.close();
                            self.mark_stopped();
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Starts the bounded fallback window after the first prompt that proves the session is real.
    pub(super) fn maybe_start_title_acquisition(&mut self, stop_reason: StopReason) {
        if !matches!(
            stop_reason,
            StopReason::EndTurn | StopReason::MaxTokens | StopReason::MaxTurnRequests
        ) {
            return;
        }
        let Some(list_supported) = self.title_acquisition.list_supported_before_prompt() else {
            return;
        };

        let final_sender = self.command_sender.clone();
        let final_handle = self
            .scheduler
            .schedule_after(Duration::from_secs(10), async move {
                if let Some(final_sender) = final_sender.upgrade() {
                    let _ = final_sender.send(RuntimeCommand::TitlePoll {
                        attempt: PollAttempt::Final,
                    });
                }
            });
        let Ok(final_handle) = final_handle else {
            self.title_acquisition.close();
            return;
        };
        let first_handle = if list_supported {
            let first_sender = self.command_sender.clone();
            match self
                .scheduler
                .schedule_after(Duration::from_secs(3), async move {
                    if let Some(first_sender) = first_sender.upgrade() {
                        let _ = first_sender.send(RuntimeCommand::TitlePoll {
                            attempt: PollAttempt::First,
                        });
                    }
                }) {
                Ok(handle) => Some(handle),
                Err(error) => {
                    ora_debug!(
                        session_id = %self.session.id,
                        error = %error,
                        "session/list first title polling was not scheduled",
                    );
                    self.title_acquisition.close();
                    return;
                }
            }
        } else {
            None
        };
        self.title_acquisition
            .start_polling(list_supported, first_handle, final_handle);
    }

    /// Accepts an ACP session-info title only while the actor's acquisition window is open.
    pub(in crate::agent_runtime) fn observe_session_update(&mut self, update: &SessionUpdate) {
        let SessionUpdate::SessionInfoUpdate(info) = update else {
            return;
        };
        let Some(title) = info.title.value() else {
            return;
        };
        self.persist_agent_title(title);
    }

    /// Locks acquisition and records the user-chosen title so later agent titles cannot win.
    pub(super) fn adopt_user_title(&mut self, title: SessionTitle) {
        self.title_acquisition.close();
        self.session = self.session.clone().with_title(Some(title.clone()));
        match self.repository.update_session_title(
            &self.session.id,
            &title,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(session) => self.session = session,
            Err(error) => {
                ora_warn!(
                    session_id = %self.session.id,
                    error = %error,
                    "failed to re-persist user session title after rename",
                );
            }
        }
    }

    /// Validates and persists a title, publishing invalidation only after the write succeeds.
    pub(super) fn persist_agent_title(&mut self, raw_title: &str) {
        if !self.title_acquisition.accepts_title() {
            return;
        }
        let title = match SessionTitle::parse(raw_title) {
            Ok(title) => title,
            Err(error) => {
                ora_warn!(
                    session_id = %self.session.id,
                    error = %error,
                    "ignored invalid agent session title",
                );
                return;
            }
        };
        if self.session.title.as_ref() == Some(&title) {
            return;
        }
        match self.repository.update_session_title(
            &self.session.id,
            &title,
            self.clock.now_timestamp_millis(),
        ) {
            Ok(session) => {
                self.session = session;
                self.app_events.try_publish(AppEvent::SessionTitleUpdated {
                    session_id: self.session.id.to_string(),
                });
            }
            Err(error) => {
                ora_warn!(
                    session_id = %self.session.id,
                    error = %error,
                    "failed to persist agent session title",
                );
            }
        }
    }
}
