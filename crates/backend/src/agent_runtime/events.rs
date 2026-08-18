use super::RuntimeActor;
use super::RuntimeCommand;
use super::connection::AgentAcpClient;
use super::routing::{SessionChannel, SessionEvent};
use crate::BackendError;
use agent_client_protocol_schema::v1::{RequestPermissionOutcome, RequestPermissionResponse};
use ora_contracts::PromptSessionEvent;
use tokio::sync::mpsc;

/// Drains an idle session's event FIFO while preserving title updates for the actor.
///
/// Idle traffic must not leak into the next prompt, but dropping `SessionInfoUpdate` here would
/// lose a push title that arrived after attach and before the first eligible prompt.
pub(super) async fn drain_idle_events(
    client: &AgentAcpClient,
    events: &mut mpsc::Receiver<SessionEvent>,
    title_updates: &mpsc::WeakUnboundedSender<RuntimeCommand>,
) {
    // Bound the synchronous snapshot so a noisy provider cannot keep an idle actor
    // from returning to its command loop forever.
    let queued = events.len();
    for _ in 0..queued {
        let Ok(event) = events.try_recv() else {
            break;
        };
        if let SessionEvent::Update(update) = &event
            && let Some(title_updates) = title_updates.upgrade()
        {
            let _ = title_updates.send(RuntimeCommand::TitleUpdate {
                update: Box::new(update.update.clone()),
            });
        }
        settle_idle_event(client, event).await;
    }
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
        SessionEvent::Update(_) | SessionEvent::Response(_) => {}
    }
}

/// Drains a cancelled prompt through its response fence while preserving its history.
pub(super) async fn settle_cancelled_prompt<Response>(
    actor: &mut RuntimeActor,
    channel: &mut SessionChannel,
    client: &AgentAcpClient,
    pending: ora_acp::PendingSessionRequest<Response>,
    events: &mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
) -> Option<Result<Response, ora_acp::AcpError>>
where
    Response: serde::de::DeserializeOwned,
{
    loop {
        match channel.events.recv().await {
            Some(SessionEvent::Update(update)) => {
                actor.observe_session_update(&update.update);
                let outcome = actor.recorder.record_update(&update.update);
                actor.settle_record(outcome);
                let _ = events.try_send(Ok(PromptSessionEvent::SessionUpdate {
                    update: update.update,
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
pub(super) async fn drain_queued_prompt_events(
    actor: &mut RuntimeActor,
    channel: &mut SessionChannel,
    client: &AgentAcpClient,
    events: &mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
) {
    // Only drain the snapshot that was accepted before isolation was requested. Events
    // arriving after this point belong to the abandoned provider request.
    let queued = channel.events.len();
    for _ in 0..queued {
        let Ok(event) = channel.events.try_recv() else {
            break;
        };
        match event {
            SessionEvent::Update(update) => {
                actor.observe_session_update(&update.update);
                let outcome = actor.recorder.record_update(&update.update);
                actor.settle_record(outcome);
                let _ = events.try_send(Ok(PromptSessionEvent::SessionUpdate {
                    update: update.update,
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

/// Drains an abandoned session request until its own response arrives or the route closes.
pub(super) async fn settle_abandoned_session_response<Response>(
    actor: &mut RuntimeActor,
    channel: &mut SessionChannel,
    client: &AgentAcpClient,
    pending: ora_acp::PendingSessionRequest<Response>,
) -> Option<Result<Response, ora_acp::AcpError>>
where
    Response: serde::de::DeserializeOwned,
{
    loop {
        match channel.events.recv().await {
            Some(SessionEvent::Update(update)) => actor.observe_session_update(&update.update),
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
