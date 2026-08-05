use super::RuntimeActor;
use super::routing::{SessionChannel, SessionEvent};
use crate::BackendError;
use ora_acp::AcpClient;
use ora_contracts::PromptSessionEvent;
use ora_contracts::acp::permission::{RequestPermissionOutcome, RequestPermissionResponse};
use tokio::process::ChildStdin;
use tokio::sync::mpsc;

/// Drains an idle session's event FIFO so stale traffic cannot cross into a later turn.
pub(super) async fn drain_idle_events(
    client: &AcpClient<ChildStdin>,
    events: &mut mpsc::Receiver<SessionEvent>,
) {
    // Bound the synchronous snapshot so a noisy provider cannot keep an idle actor
    // from returning to its command loop forever.
    let queued = events.len();
    for _ in 0..queued {
        let Ok(event) = events.try_recv() else {
            break;
        };
        settle_idle_event(client, event).await;
    }
}

/// Settles one unexpected idle event without allowing it to leak into a later operation.
pub(super) async fn settle_idle_event(client: &AcpClient<ChildStdin>, event: SessionEvent) {
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
    client: &AcpClient<ChildStdin>,
    pending: ora_acp::PendingSessionRequest<Response>,
    events: &mpsc::Sender<Result<PromptSessionEvent, BackendError>>,
) -> Option<Result<Response, ora_acp::AcpError>>
where
    Response: serde::de::DeserializeOwned,
{
    loop {
        match channel.events.recv().await {
            Some(SessionEvent::Update(update)) => {
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
    client: &AcpClient<ChildStdin>,
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
    channel: &mut SessionChannel,
    client: &AcpClient<ChildStdin>,
    pending: ora_acp::PendingSessionRequest<Response>,
) -> Option<Result<Response, ora_acp::AcpError>>
where
    Response: serde::de::DeserializeOwned,
{
    loop {
        match channel.events.recv().await {
            Some(SessionEvent::Update(_)) => {}
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
