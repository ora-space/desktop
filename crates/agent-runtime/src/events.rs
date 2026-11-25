use super::RuntimeActor;
use super::RuntimeCommand;
use super::connection::AgentAcpClient;
use super::routing::{SessionChannel, SessionEvent};
use crate::AgentRuntimeHost;
use crate::RuntimeError;
use crate::actor::actor_prompt::produced_output;
use agent_client_protocol_schema::v1::{
    RequestPermissionOutcome, RequestPermissionResponse, SessionUpdate,
};
use ora_contracts::PromptSessionEvent;
use tokio::sync::mpsc;

/// Drains an idle session's event FIFO while preserving titles for the actor and the command
/// catalog for the next prompt.
///
/// Idle traffic must not leak into the next prompt, but dropping `SessionInfoUpdate` here would
/// lose a push title that arrived after attach and before the first eligible prompt, and dropping
/// `AvailableCommandsUpdate` would lose the only report an Agent sends after answering setup.
///
/// Returns whether any drained event was turn traffic, so an idle actor can report it.
pub(super) async fn drain_idle_events(
    channel: &mut SessionChannel,
    title_updates: &mpsc::WeakUnboundedSender<RuntimeCommand>,
) -> bool {
    let client = channel.connection.client.clone();
    let mut traffic = false;
    // Bound the synchronous snapshot so a noisy provider cannot keep an idle actor
    // from returning to its command loop forever.
    let queued = channel.events.len();
    for _ in 0..queued {
        let Ok(event) = channel.events.try_recv() else {
            break;
        };
        traffic |= is_turn_traffic(&event);
        preserve_idle_update(channel, &event, title_updates);
        settle_idle_event(&client, event).await;
    }
    traffic
}

/// Keeps the parts of one idle event that outlive it, before the event itself is settled.
pub(super) fn preserve_idle_update(
    channel: &mut SessionChannel,
    event: &SessionEvent,
    title_updates: &mpsc::WeakUnboundedSender<RuntimeCommand>,
) {
    let SessionEvent::Update(update, _) = event else {
        return;
    };
    channel.retain_commands(update);
    if let Some(title_updates) = title_updates.upgrade() {
        let _ = title_updates.send(RuntimeCommand::TitleUpdate {
            update: Box::new(update.update.clone()),
        });
    }
}

/// Whether an event belongs to a turn rather than to the session.
///
/// Session-level notifications (title, available commands, modes, config options, usage) are
/// sent between turns by design and do not count; every other update, including kinds this
/// runtime does not know, and every permission request does.
pub(super) fn is_turn_traffic(event: &SessionEvent) -> bool {
    match event {
        SessionEvent::Update(update, _) => !session_level(&update.update),
        SessionEvent::Permission(_) => true,
        SessionEvent::Response(_) => false,
    }
}

/// Whether an update describes the session rather than any turn.
fn session_level(update: &SessionUpdate) -> bool {
    matches!(
        update,
        SessionUpdate::SessionInfoUpdate(_)
            | SessionUpdate::AvailableCommandsUpdate(_)
            | SessionUpdate::CurrentModeUpdate(_)
            | SessionUpdate::ConfigOptionUpdate(_)
            | SessionUpdate::UsageUpdate(_)
    )
}

/// Settles one unexpected idle event without allowing it to leak into a later operation.
pub(super) async fn settle_idle_event(client: &AgentAcpClient, event: SessionEvent) {
    match event {
        SessionEvent::Permission(permission) => {
            let _ = client
                .respond(
                    &permission.request_id,
                    &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                )
                .await;
        }
        SessionEvent::Update(_, _) | SessionEvent::Response(_) => {}
    }
}

/// Drains a cancelled prompt through its response fence while preserving its history.
pub(super) async fn settle_cancelled_prompt<Response, H: AgentRuntimeHost>(
    actor: &mut RuntimeActor<H>,
    channel: &mut SessionChannel,
    client: &AgentAcpClient,
    pending: ora_acp::PendingSessionRequest<Response>,
    events: &mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
    saw_output: &mut bool,
    followers: &mut crate::session_followers::SessionFollowers,
) -> Option<Result<Response, ora_acp::AcpError>>
where
    Response: serde::de::DeserializeOwned,
{
    loop {
        match channel.events.recv().await {
            Some(SessionEvent::Update(update, _)) => {
                *saw_output |= produced_output(&update.update);
                channel.observe_commands(&update.update);
                actor.observe_session_update(&update.update);
                let outcome = actor.recorder.record_update(&update.update);
                actor.settle_record(outcome);
                followers.send_update(&update.update, None);
                let _ = events.try_send(Ok(PromptSessionEvent::SessionUpdate {
                    update: update.update,
                    tool_timing: None,
                }));
            }
            Some(SessionEvent::Permission(permission)) => {
                let _ = client
                    .respond(
                        &permission.request_id,
                        &RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                    )
                    .await;
            }
            Some(SessionEvent::Response(response)) => {
                if !pending.matches_response(&response) {
                    continue;
                }
                return Some(pending.finish(response));
            }
            None => return None,
        }
    }
}

/// Records events already queued when cancellation settlement exceeds its grace period.
pub(super) async fn drain_queued_prompt_events<H: AgentRuntimeHost>(
    actor: &mut RuntimeActor<H>,
    channel: &mut SessionChannel,
    client: &AgentAcpClient,
    events: &mpsc::Sender<Result<PromptSessionEvent, RuntimeError>>,
    saw_output: &mut bool,
    followers: &mut crate::session_followers::SessionFollowers,
) {
    // Only drain the snapshot that was accepted before isolation was requested. Events
    // arriving after this point belong to the abandoned provider request.
    let queued = channel.events.len();
    for _ in 0..queued {
        let Ok(event) = channel.events.try_recv() else {
            break;
        };
        match event {
            SessionEvent::Update(update, _) => {
                *saw_output |= produced_output(&update.update);
                actor.observe_session_update(&update.update);
                let outcome = actor.recorder.record_update(&update.update);
                actor.settle_record(outcome);
                followers.send_update(&update.update, None);
                let _ = events.try_send(Ok(PromptSessionEvent::SessionUpdate {
                    update: update.update,
                    tool_timing: None,
                }));
            }
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
}
