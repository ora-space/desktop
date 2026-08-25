use crate::agent_runtime::SessionEventStream;
use crate::{BackendError, ErrorClassification};
use ora_contracts::{AppEvent, EmptyErrorParams, PublicError};
use ora_domain::PluginId;
use ora_logging::ora_debug;
use ora_plugin_lifecycle::PluginStatusPublisher;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

const APP_EVENT_BROADCAST_CAPACITY: usize = 64;
const APP_EVENT_STREAM_CAPACITY: usize = 64;

/// Owns best-effort application invalidations shared by every active subscriber.
#[derive(Clone)]
pub struct AppEventHub {
    events: broadcast::Sender<AppEvent>,
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
        Self { events }
    }

    /// Returns the publisher injected into backend-owned actors.
    pub(crate) fn publisher(&self) -> AppEventPublisher {
        AppEventPublisher {
            events: self.events.clone(),
        }
    }

    /// Returns an application stream that begins with Ready and receives future broadcasts.
    pub fn subscribe(&self) -> SessionEventStream<AppEvent> {
        let cancellation = CancellationToken::new();
        let receiver = self.events.subscribe();
        let (stream_sender, stream_receiver) = mpsc::channel(APP_EVENT_STREAM_CAPACITY);
        let _ = stream_sender.try_send(Ok(AppEvent::Ready));
        let forward_cancellation = cancellation.clone();
        let forward_sender = stream_sender;
        tokio::spawn(forward_events(
            receiver,
            forward_sender,
            forward_cancellation,
        ));

        SessionEventStream::with_cleanup(stream_receiver, move || {
            cancellation.cancel();
        })
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

impl PluginStatusPublisher for AppEventPublisher {
    /// Projects lifecycle invalidations onto the existing application event stream.
    fn publish_status_changed(&self, plugin_id: &PluginId) {
        self.try_publish(AppEvent::PluginStatusChanged {
            plugin_id: plugin_id.to_string(),
        });
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
                let _ = sender.try_send(Err(stream_failure("application event stream lagged")));
                return;
            }
            Err(broadcast::error::RecvError::Closed) => {
                let _ = sender.try_send(Err(stream_failure("application event hub was closed")));
                return;
            }
        }
    }
}

/// Creates the local terminal failure used when an app-event stream loses its event window.
fn stream_failure(context: &'static str) -> BackendError {
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
        let mut stream = hub.subscribe();
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

    /// Verifies application invalidations are broadcast to every active subscriber.
    #[tokio::test]
    async fn broadcasts_app_events_to_multiple_subscribers() {
        let hub = AppEventHub::new();
        let mut first = hub.subscribe();
        let mut second = hub.subscribe();
        assert_eq!(first.recv().await.unwrap().unwrap(), AppEvent::Ready);
        assert_eq!(second.recv().await.unwrap().unwrap(), AppEvent::Ready);

        let event = AppEvent::SessionTitleUpdated {
            session_id: "session-1".to_string(),
        };
        hub.publisher().try_publish(event.clone());

        assert_eq!(first.recv().await.unwrap().unwrap(), event);
        assert_eq!(second.recv().await.unwrap().unwrap(), event);
    }

    /// Verifies lifecycle publishers project plugin identifiers onto the shared event stream.
    #[tokio::test]
    async fn publishes_plugin_status_invalidations() {
        let hub = AppEventHub::new();
        let mut stream = hub.subscribe();
        assert_eq!(stream.recv().await.unwrap().unwrap(), AppEvent::Ready);

        hub.publisher()
            .publish_status_changed(&PluginId::new("official", "ora.example").expect("plugin id"));

        assert_eq!(
            stream.recv().await.unwrap().unwrap(),
            AppEvent::PluginStatusChanged {
                plugin_id: "official/ora.example".to_string(),
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

        let mut stream = hub.subscribe();
        assert_eq!(stream.recv().await.unwrap().unwrap(), AppEvent::Ready);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.recv())
                .await
                .is_err()
        );
    }
}
