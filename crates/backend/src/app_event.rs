use crate::agent_runtime::SessionEventStream;
use crate::{BackendError, ErrorClassification};
use ora_contracts::{AppEvent, EmptyErrorParams, PublicError};
use ora_logging::ora_debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

const APP_EVENT_BROADCAST_CAPACITY: usize = 64;
const APP_EVENT_STREAM_CAPACITY: usize = 64;

/// Owns best-effort application invalidations and the one active client lease.
#[derive(Clone)]
pub struct AppEventHub {
    inner: Arc<AppEventHubInner>,
}

struct AppEventHubInner {
    events: broadcast::Sender<AppEvent>,
    active_client: Mutex<Option<ActiveClient>>,
    next_generation: AtomicU64,
}

#[derive(Debug)]
struct ActiveClient {
    client_instance_id: String,
    generation: u64,
    cancellation: CancellationToken,
}

/// Provides the actor-facing, non-blocking side of the application event hub.
#[derive(Clone)]
pub(crate) struct AppEventPublisher {
    events: broadcast::Sender<AppEvent>,
}

impl AppEventHub {
    /// Creates an empty hub with bounded broadcast and per-client queues.
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(APP_EVENT_BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(AppEventHubInner {
                events,
                active_client: Mutex::new(None),
                next_generation: AtomicU64::new(1),
            }),
        }
    }

    /// Returns the publisher injected into backend-owned actors.
    pub(crate) fn publisher(&self) -> AppEventPublisher {
        AppEventPublisher {
            events: self.inner.events.clone(),
        }
    }

    /// Acquires the single-client lease and returns the first-ready application stream.
    pub fn subscribe(
        &self,
        client_instance_id: impl Into<String>,
    ) -> Result<SessionEventStream<AppEvent>, BackendError> {
        let client_instance_id = client_instance_id.into();
        if client_instance_id.trim().is_empty() {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "client instance id must not be blank",
            ));
        }

        let (generation, cancellation) = self.acquire_client(&client_instance_id)?;
        let receiver = self.inner.events.subscribe();
        let (stream_sender, stream_receiver) = mpsc::channel(APP_EVENT_STREAM_CAPACITY);
        let _ = stream_sender.try_send(Ok(AppEvent::Ready));
        let forward_cancellation = cancellation.clone();
        let forward_sender = stream_sender;
        tokio::spawn(forward_events(
            receiver,
            forward_sender,
            forward_cancellation,
        ));

        let inner = Arc::downgrade(&self.inner);
        Ok(SessionEventStream::with_cleanup(
            stream_receiver,
            move || {
                cancellation.cancel();
                release_client(&inner, &client_instance_id, generation);
            },
        ))
    }

    /// Reserves a generation for one client instance without holding the lock across I/O.
    fn acquire_client(
        &self,
        client_instance_id: &str,
    ) -> Result<(u64, CancellationToken), BackendError> {
        let mut active_client = self
            .inner
            .active_client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(active) = active_client.as_ref()
            && active.client_instance_id != client_instance_id
        {
            return Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::MultipleClientsUnsupported(EmptyErrorParams {}),
                "another application client already owns the backend",
            ));
        }

        // A same-document reconnect replaces the transport but not the logical client. Cancel
        // the old forwarder while holding the lease lock so its eventual Drop cannot overlap the
        // new generation or release a later client's ownership.
        if let Some(active) = active_client.as_ref() {
            active.cancellation.cancel();
        }
        let generation = self.inner.next_generation.fetch_add(1, Ordering::Relaxed);
        let cancellation = CancellationToken::new();
        *active_client = Some(ActiveClient {
            client_instance_id: client_instance_id.to_owned(),
            generation,
            cancellation: cancellation.clone(),
        });
        Ok((generation, cancellation))
    }
}

impl Default for AppEventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl AppEventPublisher {
    /// Publishes one invalidation without blocking the actor that changed durable state.
    pub(crate) fn try_publish(&self, event: AppEvent) {
        if self.events.send(event).is_err() {
            ora_debug!("application event dropped because no client is subscribed");
        }
    }
}

/// Forwards broadcast events through a bounded queue so a slow client cannot block publishers.
async fn forward_events(
    mut receiver: broadcast::Receiver<AppEvent>,
    sender: mpsc::Sender<Result<AppEvent, BackendError>>,
    cancellation: CancellationToken,
) {
    loop {
        let event = tokio::select! {
            _ = cancellation.cancelled() => return,
            event = receiver.recv() => event,
        };
        match event {
            Ok(event) => {
                if sender.try_send(Ok(event)).is_err() {
                    ora_debug!("application event stream queue overflowed");
                    return;
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                ora_debug!(skipped, "application event subscriber lagged");
                let _ = sender.try_send(Err(stream_interrupted("application event stream lagged")));
                return;
            }
            Err(broadcast::error::RecvError::Closed) => {
                let _ =
                    sender.try_send(Err(stream_interrupted("application event hub was closed")));
                return;
            }
        }
    }
}

/// Releases a lease only when it still belongs to the current stream generation.
fn release_client(inner: &Weak<AppEventHubInner>, client_instance_id: &str, generation: u64) {
    let Some(inner) = inner.upgrade() else {
        return;
    };
    let mut active_client = inner
        .active_client
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if active_client.as_ref().is_some_and(|active| {
        active.generation == generation && active.client_instance_id == client_instance_id
    }) {
        *active_client = None;
    }
}

/// Creates the local terminal failure used when an app-event stream loses its event window.
fn stream_interrupted(context: &'static str) -> BackendError {
    BackendError::new(
        ErrorClassification::Internal,
        PublicError::InternalError(EmptyErrorParams {}),
        context,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    /// Verifies every new subscription receives Ready before published invalidations.
    #[tokio::test]
    async fn sends_ready_before_app_events() {
        let hub = AppEventHub::new();
        let mut stream = hub
            .subscribe("client-a")
            .expect("client lease is available");
        assert_eq!(
            stream
                .recv()
                .await
                .expect("Ready frame is present")
                .expect("Ready frame is not an error"),
            AppEvent::Ready,
        );

        hub.publisher().try_publish(AppEvent::SessionTitleUpdated {
            session_id: "session-1".to_string(),
        });
        assert_eq!(
            stream
                .recv()
                .await
                .expect("title event is present")
                .expect("title event is not an error"),
            AppEvent::SessionTitleUpdated {
                session_id: "session-1".to_string(),
            },
        );
    }

    /// Verifies best-effort events published without a subscriber are not replayed later.
    #[tokio::test]
    async fn drops_events_when_no_client_is_subscribed() {
        let hub = AppEventHub::new();
        hub.publisher().try_publish(AppEvent::SessionTitleUpdated {
            session_id: "session-before-watch".to_string(),
        });

        let mut stream = hub
            .subscribe("client-a")
            .expect("client lease is available");
        assert_eq!(stream.recv().await.unwrap().unwrap(), AppEvent::Ready);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.recv())
                .await
                .is_err()
        );
    }

    /// Verifies a different client instance is rejected until the active stream is dropped.
    #[tokio::test]
    async fn enforces_single_client_ownership() {
        let hub = AppEventHub::new();
        let stream = hub
            .subscribe("client-a")
            .expect("client lease is available");
        let error = match hub.subscribe("client-b") {
            Ok(_) => panic!("a second client must be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.public_error(),
            &PublicError::MultipleClientsUnsupported(EmptyErrorParams {}),
        );
        drop(stream);
        assert!(hub.subscribe("client-b").is_ok());
    }

    /// Verifies an old stream cleanup cannot release a newer stream for the same document.
    #[tokio::test]
    async fn reconnect_generation_protects_new_lease() {
        let hub = AppEventHub::new();
        let old_stream = hub
            .subscribe("client-a")
            .expect("client lease is available");
        let new_stream = hub
            .subscribe("client-a")
            .expect("same document may reconnect");
        drop(old_stream);
        assert!(hub.subscribe("client-b").is_err());
        drop(new_stream);
        assert!(hub.subscribe("client-b").is_ok());
    }

    /// Verifies a same-document reconnect terminates the superseded stream.
    #[tokio::test]
    async fn reconnect_cancels_old_stream() {
        let hub = AppEventHub::new();
        let mut old_stream = hub
            .subscribe("client-a")
            .expect("client lease is available");
        assert_eq!(old_stream.recv().await.unwrap().unwrap(), AppEvent::Ready);

        let mut new_stream = hub
            .subscribe("client-a")
            .expect("same document may reconnect");
        assert_eq!(new_stream.recv().await.unwrap().unwrap(), AppEvent::Ready);

        assert!(
            tokio::time::timeout(Duration::from_secs(1), old_stream.recv())
                .await
                .expect("old stream should terminate promptly")
                .is_none()
        );
        drop(new_stream);
    }

    /// Verifies the old stream cannot release another client after the replacement disconnects.
    #[tokio::test]
    async fn old_stream_drop_cannot_release_client_after_new_stream_drop() {
        let hub = AppEventHub::new();
        let old_stream = hub
            .subscribe("client-a")
            .expect("client lease is available");
        let new_stream = hub
            .subscribe("client-a")
            .expect("same document may reconnect");
        drop(new_stream);

        let client_b = hub
            .subscribe("client-b")
            .expect("replacement client may claim a released lease");
        drop(old_stream);

        assert!(hub.subscribe("client-c").is_err());
        drop(client_b);
    }
}
