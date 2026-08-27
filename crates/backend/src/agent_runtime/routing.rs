use super::connection::RuntimeConnection;
use crate::BackendError;
use agent_client_protocol_schema::v1::SessionNotification;
use ora_acp::{PermissionRequest, SessionResponse, SessionTraceRegistration};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

/// Carries normal session traffic in the same order observed on the ACP connection.
#[derive(Debug)]
pub(super) enum SessionEvent {
    Update(SessionNotification),
    Permission(PermissionRequest),
    Response(SessionResponse),
}

impl SessionEvent {
    /// Returns the provider session id used to select the destination route.
    fn session_id(&self) -> &str {
        match self {
            Self::Update(update) => &update.session_id.0,
            Self::Permission(permission) => &permission.request.session_id.0,
            Self::Response(response) => &response.session_id().0,
        }
    }
}

/// Carries failures that must remain observable when the normal event queue is full.
pub(super) enum SessionControl {
    ConnectionLost(BackendError),
    QueueOverflow,
}

/// Owns one session's generation-bound routes on the shared ACP connection.
pub(super) struct SessionChannel {
    pub connection: RuntimeConnection,
    pub events: mpsc::Receiver<SessionEvent>,
    pub pending_updates: VecDeque<SessionNotification>,
    pub controls: mpsc::UnboundedReceiver<SessionControl>,
    pub(super) _trace_registration: SessionTraceRegistration,
    pub(super) _registration: RouteRegistration,
}

#[derive(Default)]
pub(super) struct RouteRegistry {
    entries: RwLock<HashMap<String, RouteEntry>>,
    next_token: AtomicU64,
    setup_count: AtomicU64,
    pending_setup_updates: Mutex<VecDeque<SessionNotification>>,
}

struct RouteEntry {
    generation: u64,
    token: u64,
    events: mpsc::Sender<SessionEvent>,
    controls: mpsc::UnboundedSender<SessionControl>,
}

impl RouteRegistry {
    /// Keeps unrouted notifications briefly while `session/new` reveals its provider id.
    pub(super) fn begin_session_setup(self: &Arc<Self>) -> SetupRegistration {
        self.setup_count.fetch_add(1, Ordering::AcqRel);
        SetupRegistration {
            registry: self.clone(),
        }
    }

    /// Installs a route token so a stale actor cannot unregister a newer generation.
    pub(super) fn register(
        self: &Arc<Self>,
        session_id: &str,
        generation: u64,
        events: mpsc::Sender<SessionEvent>,
        controls: mpsc::UnboundedSender<SessionControl>,
    ) -> RouteRegistration {
        // Serializing registration with setup buffering closes the race where an
        // update is classified as unrouted immediately before this route appears.
        let mut pending = self
            .pending_setup_updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let token = self.next_token.fetch_add(1, Ordering::Relaxed) + 1;
        self.write_entries().insert(
            session_id.to_string(),
            RouteEntry {
                generation,
                token,
                events: events.clone(),
                controls,
            },
        );
        let mut retained = VecDeque::new();
        while let Some(update) = pending.pop_front() {
            if update.session_id.0.as_ref() == session_id {
                let _ = events.try_send(SessionEvent::Update(update));
            } else {
                retained.push_back(update);
            }
        }
        *pending = retained;
        RouteRegistration {
            session_id: session_id.to_string(),
            token,
            registry: self.clone(),
        }
    }

    /// Routes one ordered event without allowing a slow session to poison the connection.
    pub(super) fn route_event(&self, event: SessionEvent) -> Result<(), Box<SessionEvent>> {
        let mut pending = self
            .pending_setup_updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let session_id = event.session_id().to_string();
        let route = self
            .read_entries()
            .get(&session_id)
            .map(|entry| (entry.token, entry.events.clone()));
        match route {
            Some((token, events)) => match events.try_send(event) {
                Err(TrySendError::Full(event)) => {
                    if let Some(entry) = self.remove_route(&session_id, token) {
                        let _ = entry.controls.send(SessionControl::QueueOverflow);
                    }
                    Err(Box::new(event))
                }
                Err(TrySendError::Closed(event)) => {
                    self.remove_route(&session_id, token);
                    Err(Box::new(event))
                }
                Ok(()) => Ok(()),
            },
            None if self.setup_count.load(Ordering::Acquire) > 0 => match event {
                SessionEvent::Update(update) => {
                    if pending.len() == super::CONTRACT_QUEUE_CAPACITY {
                        pending.pop_front();
                    }
                    pending.push_back(update);
                    Ok(())
                }
                SessionEvent::Permission(_) | SessionEvent::Response(_) => Err(Box::new(event)),
            },
            None => Err(Box::new(event)),
        }
    }

    /// Invalidates every route owned by a failed connection generation.
    pub(super) fn fail_generation(&self, generation: u64, error: BackendError) {
        let failed = {
            let mut entries = self.write_entries();
            let all = std::mem::take(&mut *entries);
            let (failed, retained): (HashMap<_, _>, HashMap<_, _>) = all
                .into_iter()
                .partition(|(_, entry)| entry.generation == generation);
            *entries = retained;
            failed
        };
        for entry in failed.into_values() {
            let _ = entry
                .controls
                .send(SessionControl::ConnectionLost(error.clone()));
        }
    }

    /// Removes only the route observed by a delivery attempt, preserving a newer registration.
    fn remove_route(&self, session_id: &str, token: u64) -> Option<RouteEntry> {
        let mut entries = self.write_entries();
        if entries
            .get(session_id)
            .is_some_and(|entry| entry.token == token)
        {
            entries.remove(session_id)
        } else {
            None
        }
    }

    /// Recovers a poisoned read lock because route loss is safer than crashing the supervisor.
    fn read_entries(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, RouteEntry>> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Recovers a poisoned write lock so connection recovery can still invalidate stale routes.
    fn write_entries(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, RouteEntry>> {
        self.entries.write().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(super) struct RouteRegistration {
    session_id: String,
    token: u64,
    registry: Arc<RouteRegistry>,
}

/// Bounds the lifetime in which an as-yet-unknown session id may emit setup updates.
pub(super) struct SetupRegistration {
    registry: Arc<RouteRegistry>,
}

impl Drop for SetupRegistration {
    fn drop(&mut self) {
        if self.registry.setup_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.registry
                .pending_setup_updates
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
        }
    }
}

impl Drop for RouteRegistration {
    fn drop(&mut self) {
        self.registry.remove_route(&self.session_id, self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::{RouteRegistry, SessionControl, SessionEvent};
    use agent_client_protocol_schema::v1::RequestId;
    use agent_client_protocol_schema::v1::RequestPermissionRequest;
    use agent_client_protocol_schema::v1::SessionId;
    use agent_client_protocol_schema::v1::SessionNotification;
    use agent_client_protocol_schema::v1::{SessionInfoUpdate, SessionUpdate};
    use agent_client_protocol_schema::v1::{ToolCallUpdate, ToolCallUpdateFields};
    use ora_acp::{
        AcpError, AcpInboundEvent, AcpPeer, AcpTransport, PermissionRequest, SessionResponse,
    };
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// Records what one connection wrote, standing in for the plugin transport these tests do not
    /// need to exercise.
    struct RecordingTransport {
        sent: mpsc::UnboundedSender<Value>,
    }

    impl AcpTransport for RecordingTransport {
        async fn send(&self, message: Value) -> Result<(), AcpError> {
            let _ = self.sent.send(message);
            Ok(())
        }
    }

    /// Produces one genuine terminating response for `session_id`.
    ///
    /// Routing has to be asserted on a real `SessionResponse`, which only the ACP peer can mint:
    /// it carries the correlation the peer established when the request went out, and no
    /// constructor exposes that from here.
    async fn session_response(session_id: &str) -> SessionResponse {
        let (inbound, messages) = mpsc::unbounded_channel();
        let (sent, mut outbound) = mpsc::unbounded_channel();
        let mut peer = AcpPeer::spawn(messages, RecordingTransport { sent });
        let session_id = SessionId::new(session_id);
        let _pending = peer
            .client
            .start_session_request::<_, Value>(
                session_id.clone(),
                "session/prompt",
                &json!({ "sessionId": session_id }),
            )
            .await
            .expect("start session request");
        let request = outbound.recv().await.expect("receive session request");
        inbound
            .send(Ok(json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "result": { "stopReason": "end_turn" },
            })))
            .expect("queue session response");
        match peer.next_event().await.expect("receive response event") {
            AcpInboundEvent::SessionResponse(response) => response,
            AcpInboundEvent::SessionUpdate(_)
            | AcpInboundEvent::PermissionRequest(_)
            | AcpInboundEvent::Fatal(_) => panic!("expected session response"),
        }
    }

    /// Verifies central routing keeps concurrent session update streams isolated.
    #[tokio::test]
    async fn routes_updates_only_to_the_matching_session() {
        let routes = Arc::new(RouteRegistry::default());
        let (first_updates, mut first_receiver) = mpsc::channel(1);
        let (first_controls, _first_controls_receiver) = mpsc::unbounded_channel();
        let (second_updates, mut second_receiver) = mpsc::channel(1);
        let (second_controls, _second_controls_receiver) = mpsc::unbounded_channel();
        let _first = routes.register("session-1", 1, first_updates, first_controls);
        let _second = routes.register("session-2", 1, second_updates, second_controls);
        let update = SessionNotification::new(
            "session-1",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("First")),
        );

        assert!(
            routes
                .route_event(SessionEvent::Update(update.clone()))
                .is_ok()
        );

        match first_receiver.recv().await {
            Some(SessionEvent::Update(received)) => assert_eq!(received, update),
            Some(SessionEvent::Permission(_) | SessionEvent::Response(_)) | None => {
                panic!("expected session update")
            }
        }
        assert!(second_receiver.try_recv().is_err());
    }

    /// Verifies a terminating response cannot overtake an update in the per-session FIFO.
    #[tokio::test]
    async fn preserves_update_then_response_order() {
        let response = session_response("session-1").await;
        let routes = Arc::new(RouteRegistry::default());
        let (events, mut events_receiver) = mpsc::channel(2);
        let (controls, _controls_receiver) = mpsc::unbounded_channel();
        let _registration = routes.register("session-1", 1, events, controls);
        let update = SessionNotification::new(
            "session-1",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new()),
        );

        assert!(
            routes
                .route_event(SessionEvent::Update(update.clone()))
                .is_ok()
        );
        assert!(routes.route_event(SessionEvent::Response(response)).is_ok());

        match events_receiver.recv().await {
            Some(SessionEvent::Update(received)) => assert_eq!(received, update),
            Some(SessionEvent::Permission(_) | SessionEvent::Response(_)) | None => {
                panic!("expected session update")
            }
        }
        assert!(matches!(
            events_receiver.recv().await,
            Some(SessionEvent::Response(_))
        ));
    }

    /// Verifies an orphan permission is returned so the connection loop can cancel it immediately.
    #[test]
    fn returns_a_permission_that_has_no_session_route() {
        let routes = RouteRegistry::default();
        let permission = PermissionRequest {
            request_id: RequestId::Number(7),
            request: RequestPermissionRequest::new(
                "missing-session",
                ToolCallUpdate::new("tool-1", ToolCallUpdateFields::new()),
                Vec::new(),
            ),
        };

        match routes.route_event(SessionEvent::Permission(permission.clone())) {
            Err(event) => match *event {
                SessionEvent::Permission(received) => assert_eq!(received, permission),
                SessionEvent::Update(_) | SessionEvent::Response(_) => {
                    panic!("expected permission request")
                }
            },
            Ok(()) => panic!("expected missing route"),
        }
    }

    /// Verifies session/new updates survive until the provider id can be registered.
    #[tokio::test]
    async fn buffers_updates_during_session_setup() {
        let routes = Arc::new(RouteRegistry::default());
        let setup = routes.begin_session_setup();
        let update = SessionNotification::new(
            "new-session",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Created")),
        );
        assert!(
            routes
                .route_event(SessionEvent::Update(update.clone()))
                .is_ok()
        );
        let (updates, mut receiver) = mpsc::channel(1);
        let (controls, _controls_receiver) = mpsc::unbounded_channel();

        let _registration = routes.register("new-session", 1, updates, controls);
        drop(setup);

        match receiver.recv().await {
            Some(SessionEvent::Update(received)) => assert_eq!(received, update),
            Some(SessionEvent::Permission(_) | SessionEvent::Response(_)) | None => {
                panic!("expected setup update")
            }
        }
    }

    /// Verifies one slow session is detached without invalidating unrelated routes.
    #[tokio::test]
    async fn isolates_a_session_whose_update_queue_overflows() {
        let routes = Arc::new(RouteRegistry::default());
        let (updates, _updates_receiver) = mpsc::channel(1);
        let (controls, mut controls_receiver) = mpsc::unbounded_channel();
        let _registration = routes.register("session-1", 1, updates, controls);
        let update = SessionNotification::new(
            "session-1",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new()),
        );

        assert!(
            routes
                .route_event(SessionEvent::Update(update.clone()))
                .is_ok()
        );
        assert!(routes.route_event(SessionEvent::Update(update)).is_err());

        assert!(matches!(
            controls_receiver.recv().await,
            Some(SessionControl::QueueOverflow)
        ));
        assert!(!routes.read_entries().contains_key("session-1"));
    }

    /// Verifies a connection failure invalidates only routes from its generation.
    #[tokio::test]
    async fn invalidates_only_the_failed_connection_generation() {
        let routes = Arc::new(RouteRegistry::default());
        let (old_updates, _old_updates_receiver) = mpsc::channel(1);
        let (old_controls, mut old_controls_receiver) = mpsc::unbounded_channel();
        let (new_updates, _new_updates_receiver) = mpsc::channel(1);
        let (new_controls, mut new_controls_receiver) = mpsc::unbounded_channel();
        let _old = routes.register("old", 1, old_updates, old_controls);
        let _new = routes.register("new", 2, new_updates, new_controls);
        let error = super::super::runtime_internal("agent_runtime_unavailable", "connection lost");

        routes.fail_generation(1, error.clone());

        match old_controls_receiver.recv().await {
            Some(SessionControl::ConnectionLost(received)) => {
                assert_eq!(received.classification(), error.classification());
                assert_eq!(received.public_error(), error.public_error());
            }
            Some(SessionControl::QueueOverflow) | None => {
                panic!("expected connection loss")
            }
        }
        assert!(new_controls_receiver.try_recv().is_err());
        assert!(routes.read_entries().contains_key("new"));
    }

    /// Verifies an event already queued remains readable after generation failure notifies loss.
    ///
    /// Active scheduling drains this queue before applying a terminal connection control.
    #[tokio::test]
    async fn keeps_queued_events_readable_after_connection_loss() {
        let response = session_response("session-1").await;
        let routes = Arc::new(RouteRegistry::default());
        let (events, mut events_receiver) = mpsc::channel(2);
        let (controls, mut controls_receiver) = mpsc::unbounded_channel();
        let _registration = routes.register("session-1", 1, events, controls);
        assert!(routes.route_event(SessionEvent::Response(response)).is_ok());
        let error = super::super::runtime_internal("agent_runtime_unavailable", "connection lost");

        routes.fail_generation(1, error);

        assert!(matches!(
            events_receiver.recv().await,
            Some(SessionEvent::Response(_))
        ));
        assert!(matches!(
            controls_receiver.recv().await,
            Some(SessionControl::ConnectionLost(_))
        ));
    }
}
