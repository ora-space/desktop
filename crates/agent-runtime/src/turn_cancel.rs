//! A caller-owned cancel of the current prompt with its own answer deadline.
//!
//! The desktop stop button cancels with the runtime's short grace and keeps whatever the provider
//! did. A Node `CancelTurn` instead names an absolute deadline (its acceptance time plus the cancel
//! grace) and must learn whether the Agent answered by then. That request is shared state rather
//! than only a command because three places must see it without waiting for the actor's command
//! loop: the gap between admission and the prompt write, the watchdog's cancel wait, and its
//! resend. The first request of a prompt fixes the deadline; later ones never move it.
use crate::actor::actor_prompt::produced_output;
use crate::connection::AgentAcpClient;
use crate::events::{drain_queued_prompt_events, settle_cancelled_prompt};
use crate::limits::CANCELLATION_GRACE;
use crate::routing::{SessionChannel, SessionEvent};
use crate::session_followers::SessionFollowers;
use crate::support::{map_prompt_error, provider_session_invalid};
use crate::tool_timing::ToolTimings;
use crate::{AgentRuntimeHost, PromptEndCause, PromptEndReport, RuntimeActor, RuntimeError};
use agent_client_protocol_schema::v1::{
    PromptResponse, RequestId, RequestPermissionOutcome, RequestPermissionResponse, StopReason,
};
use ora_contracts::PromptSessionEvent;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::{Instant, timeout_at};

/// The pending turn cancel of one session actor.
#[derive(Debug, Default)]
pub(crate) struct TurnCancelState {
    deadline: Mutex<Option<Instant>>,
    changed: Notify,
    /// Turn traffic the idle actor settled since the last prompt was admitted. An idle actor
    /// answers such traffic at once, so it is remembered rather than left in the queue.
    idle_traffic: std::sync::atomic::AtomicBool,
}

impl TurnCancelState {
    /// Records a cancel unless one is already pending; the first deadline wins.
    pub(crate) fn request(&self, deadline: Instant) {
        let mut slot = self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(deadline);
            self.changed.notify_waiters();
        }
    }

    /// The pending cancel's deadline, if one was requested for the current prompt.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Forgets a request that belonged to an earlier prompt; called when a new prompt arrives,
    /// before its admission commits, so a late cancel of the previous turn cannot stop this one.
    pub(crate) fn clear(&self) {
        *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.idle_traffic
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Remembers that the idle actor settled turn traffic.
    pub(crate) fn note_idle_traffic(&self) {
        self.idle_traffic
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether turn traffic arrived since the last prompt; reading it forgets it.
    pub(crate) fn take_idle_traffic(&self) -> bool {
        self.idle_traffic
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    /// Resolves once a cancel is requested; callers re-read [`Self::deadline`] afterwards.
    pub(crate) async fn requested(&self) {
        loop {
            let changed = self.changed.notified();
            if self.deadline().is_some() {
                return;
            }
            changed.await;
        }
    }
}

/// One prompt's state a cancel needs, borrowed from the prompt loop.
pub(crate) struct PromptStop<'a> {
    pub(crate) operation_id: u64,
    pub(crate) client: &'a AgentAcpClient,
    pub(crate) permissions: &'a HashMap<String, (RequestId, Vec<String>)>,
    pub(crate) events: &'a mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
    pub(crate) end_report: &'a PromptEndReport,
    pub(crate) followers: &'a mut SessionFollowers,
    pub(crate) tool_timings: &'a ToolTimings,
    pub(crate) saw_output: &'a mut bool,
}

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Ends a prompt that a turn cancel stopped after admission but before it was written: the
    /// Agent never receives this turn, its end is recorded as cancelled, and the provider session
    /// is untouched, so it stays reusable without any quiescence check.
    pub(crate) fn finish_unsent_prompt(
        &mut self,
        channel: SessionChannel,
        end_report: &PromptEndReport,
        events: &mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
        followers: &mut SessionFollowers,
    ) {
        end_report.record(PromptEndCause::UserStopped);
        end_report.mark_unsent();
        end_report.mark_cancel_applied();
        self.end_turn(StopReason::Cancelled);
        followers.finish(StopReason::Cancelled);
        let _ = events.try_send(Ok(PromptSessionEvent::Completed {
            stop_reason: StopReason::Cancelled,
            token_usage: None,
        }));
        self.channel = Some(channel);
    }

    /// Cancels the running prompt for the stop button (`notify_owner` with no turn cancel), a
    /// turn cancel with its own deadline, or an owner that stopped reading, and reports the end.
    ///
    /// A turn cancel first looks, without waiting, for an answer that is already readable: only
    /// that is a real race the Agent won, and its stop reason is kept. Any answer that arrives
    /// once `session/cancel` is written is the cancel's answer and is reported as `cancelled`,
    /// so the caller judges the stop (D2) instead of trusting the Agent's own `end_turn`.
    pub(crate) async fn settle_prompt_cancel(
        &mut self,
        mut channel: SessionChannel,
        pending: ora_acp::PendingSessionRequest<PromptResponse>,
        stop: PromptStop<'_>,
        notify_owner: bool,
        completion: Option<oneshot::Sender<Result<(), RuntimeError>>>,
    ) {
        let PromptStop {
            operation_id,
            client,
            permissions,
            events,
            end_report,
            followers,
            tool_timings,
            saw_output,
        } = stop;
        // A turn cancel waits until its own deadline and must report an unanswered cancel; the
        // stop button keeps the runtime's short grace.
        let turn_deadline = self.turn_cancel.deadline().filter(|_| notify_owner);
        if turn_deadline.is_some()
            && let Some(response) =
                ready_response(self, &mut channel, client, &pending, events, saw_output).await
        {
            // The prompt had already ended on its own: the cancel did not act on it, and the
            // answer keeps its own meaning, success or failure.
            self.finish_answered_prompt(
                channel,
                pending.finish(response),
                operation_id,
                stop_rest(events, end_report, followers, tool_timings),
            )
            .await;
            return;
        }
        // `cancel_prompt` is the user pressing stop, while the owner-addressed cancel is a
        // consumer that stopped reading; the two must not be reported alike.
        end_report.record(if notify_owner {
            PromptEndCause::UserStopped
        } else {
            PromptEndCause::StreamDropped
        });
        if turn_deadline.is_some() {
            end_report.mark_cancel_applied();
        }
        self.cancel(client, permissions).await;
        let settled = timeout_at(
            turn_deadline.unwrap_or_else(|| Instant::now() + CANCELLATION_GRACE),
            settle_cancelled_prompt(
                self,
                &mut channel,
                client,
                pending,
                events,
                saw_output,
                followers,
            ),
        )
        .await;
        let reusable = matches!(
            settled,
            Ok(Some(Ok(_))) | Ok(Some(Err(ora_acp::AcpError::RequestFailed(_))))
        );
        let session_invalid =
            matches!(&settled, Ok(Some(Err(error))) if provider_session_invalid(error));
        if !reusable {
            drain_queued_prompt_events(self, &mut channel, client, events, saw_output, followers)
                .await;
        }
        // Every answer once `session/cancel` is written answers the cancel.
        let stop_reason = StopReason::Cancelled;
        self.end_timed_turn(stop_reason, tool_timings);
        followers.finish(stop_reason);
        let owner_notified = !notify_owner
            || if turn_deadline.is_some() && !reusable {
                events
                    .try_send(Err(RuntimeError::turn_cancel_unanswered()))
                    .is_ok()
            } else {
                events
                    .try_send(Ok(PromptSessionEvent::Completed {
                        stop_reason,
                        token_usage: None,
                    }))
                    .is_ok()
            };
        if reusable && owner_notified && !session_invalid {
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
    }
}

/// The part of a prompt's state its own end reports into.
pub(crate) struct PromptEnd<'a> {
    pub(crate) events: &'a mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
    pub(crate) end_report: &'a PromptEndReport,
    pub(crate) followers: &'a mut SessionFollowers,
    pub(crate) tool_timings: &'a ToolTimings,
}

/// Groups the state a prompt's own end reports into.
fn stop_rest<'a>(
    events: &'a mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
    end_report: &'a PromptEndReport,
    followers: &'a mut SessionFollowers,
    tool_timings: &'a ToolTimings,
) -> PromptEnd<'a> {
    PromptEnd {
        events,
        end_report,
        followers,
        tool_timings,
    }
}

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Ends a prompt whose answer was already readable when a turn cancel was about to be
    /// written, exactly as the prompt loop ends an answered prompt: a result keeps its stop
    /// reason, an ACP error is the prompt's failure (marked answered when it is a request error).
    async fn finish_answered_prompt(
        &mut self,
        channel: SessionChannel,
        answer: Result<PromptResponse, ora_acp::AcpError>,
        operation_id: u64,
        end: PromptEnd<'_>,
    ) {
        let PromptEnd {
            events,
            end_report,
            followers,
            tool_timings,
        } = end;
        self.load_restoration = crate::attach::LoadRestoration::NotRestored;
        match answer {
            Ok(response) => {
                self.finish_completed_prompt(
                    channel,
                    response,
                    operation_id,
                    PromptEnd {
                        events,
                        end_report,
                        followers,
                        tool_timings,
                    },
                )
                .await;
            }
            Err(error) => {
                let reusable = matches!(&error, ora_acp::AcpError::RequestFailed(_));
                let session_invalid = provider_session_invalid(&error);
                if reusable {
                    self.record_cleanup(operation_id, Ok(()));
                }
                end_report.record(PromptEndCause::Unknown);
                self.end_timed_turn(StopReason::Cancelled, tool_timings);
                followers.finish(StopReason::Cancelled);
                let failure = map_prompt_error(error);
                let failure = if reusable {
                    failure.answered()
                } else {
                    failure
                };
                let delivered = events.try_send(Err(failure)).is_ok();
                if reusable && delivered && !session_invalid {
                    self.channel = Some(channel);
                } else {
                    self.isolate_channel(channel).await;
                }
            }
        }
    }
}

/// Consumes the events already queued for the prompt, recording them as the prompt loop would,
/// and returns its answer if one is among them. Nothing is awaited from the Agent.
async fn ready_response<H: AgentRuntimeHost>(
    actor: &mut RuntimeActor<H>,
    channel: &mut SessionChannel,
    client: &AgentAcpClient,
    pending: &ora_acp::PendingSessionRequest<PromptResponse>,
    events: &mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
    saw_output: &mut bool,
) -> Option<ora_acp::SessionResponse> {
    let (before, response) = take_ready_response(&mut channel.events, |response| {
        pending.matches_response(response)
    });
    for event in before {
        match event {
            SessionEvent::Update(update, _) => {
                *saw_output |= produced_output(&update.update);
                actor.observe_session_update(&update.update);
                let outcome = actor.recorder.record_update(&update.update);
                actor.settle_record(outcome);
                let _ = events.try_send(Ok(PromptSessionEvent::SessionUpdate {
                    update: update.update,
                    tool_timing: None,
                }));
            }
            // A request still open when the turn is being stopped is answered as the cancel
            // would answer it.
            SessionEvent::Permission(permission) => {
                let _ = client
                    .respond(
                        &permission.request_id,
                        &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                    )
                    .await;
            }
            SessionEvent::Response(_) => {}
        }
    }
    response
}

/// Takes the queued snapshot up to the prompt's own answer, without waiting: the events before
/// it, and the answer if it is already readable. Events after the answer stay queued.
fn take_ready_response(
    events: &mut mpsc::Receiver<SessionEvent>,
    answers: impl Fn(&ora_acp::SessionResponse) -> bool,
) -> (Vec<SessionEvent>, Option<ora_acp::SessionResponse>) {
    let mut before = Vec::new();
    for _ in 0..events.len() {
        let Ok(event) = events.try_recv() else {
            break;
        };
        match event {
            SessionEvent::Response(response) if answers(&response) => {
                return (before, Some(response));
            }
            event => before.push(event),
        }
    }
    (before, None)
}

#[cfg(test)]
mod tests {
    use super::TurnCancelState;
    use pretty_assertions::assert_eq;
    use std::time::Duration;
    use tokio::time::Instant;

    /// A second request never moves the deadline the first one fixed.
    #[tokio::test(start_paused = true)]
    async fn the_first_deadline_is_kept() {
        let state = TurnCancelState::default();
        let first = Instant::now() + Duration::from_secs(15);
        state.request(first);
        state.request(first + Duration::from_secs(10));
        assert_eq!(state.deadline(), Some(first));
        state.requested().await;
        state.clear();
        assert_eq!(state.deadline(), None);
    }

    /// Only an answer already queued counts as having won the race; events before it are
    /// returned for recording, and events after it stay queued.
    #[tokio::test]
    async fn only_an_already_queued_answer_is_taken() {
        use super::take_ready_response;
        use crate::routing::SessionEvent;
        use agent_client_protocol_schema::v1::{
            SessionInfoUpdate, SessionNotification, SessionUpdate,
        };
        let (sender, mut events) = tokio::sync::mpsc::channel(4);
        let update = || {
            SessionEvent::update(SessionNotification::new(
                "session-1",
                SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new()),
            ))
        };
        assert!(matches!(
            take_ready_response(&mut events, |_| true),
            (before, None) if before.is_empty()
        ));
        sender.try_send(update()).unwrap();
        sender
            .try_send(SessionEvent::Response(
                crate::routing::tests::session_response("session-1").await,
            ))
            .unwrap();
        sender.try_send(update()).unwrap();
        let (before, answer) = take_ready_response(&mut events, |_| true);
        assert_eq!((before.len(), answer.is_some(), events.len()), (1, true, 1));
        let (before, answer) = take_ready_response(&mut events, |_| false);
        assert_eq!((before.len(), answer.is_none()), (1, true));
    }
}
