use crate::agent_runtime::SessionEventStream;
use crate::{BackendError, ErrorClassification};
use ora_agent_runtime::RuntimeError;
use ora_contracts::{AppEvent, EmptyErrorParams, PublicError};
use ora_domain::PluginId;
use ora_logging::ora_debug;
use ora_plugin_lifecycle::PluginStatusPublisher;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

/// How many events wait for one client before its stream ends; it reconnects and re-queries.
const APP_EVENT_QUEUE_CAPACITY: usize = 64;
const APP_EVENT_STREAM_CAPACITY: usize = 64;

/// Owns best-effort application invalidations shared by every active subscriber.
#[derive(Clone)]
pub struct AppEventHub {
    subscribers: Arc<Subscribers>,
}

/// Provides the actor-facing, non-blocking side of the application event hub.
#[derive(Clone)]
pub(crate) struct AppEventPublisher {
    subscribers: Arc<Subscribers>,
}

/// The queues of the clients subscribed now; a client that went away leaves a dead entry behind.
#[derive(Default)]
struct Subscribers {
    queues: Mutex<Vec<Weak<ClientQueue>>>,
}

/// The events one client has not taken yet.
///
/// Publishers append here directly, so a burst published before the client's forwarder runs is
/// merged and bounded here rather than lost: an invalidation already waiting stands for a new one
/// of the same kind, which is how a batch of Skill changes, each told to every agent, stays within
/// the capacity.
#[derive(Default)]
struct ClientQueue {
    state: Mutex<QueueState>,
    changed: Notify,
}

#[derive(Default)]
struct QueueState {
    events: VecDeque<AppEvent>,
    /// Why the stream ends after the events before it: nothing more is queued then.
    end: Option<StreamEnd>,
}

/// Why a client's stream stopped taking events.
#[derive(Clone, Copy)]
enum StreamEnd {
    /// More events waited than the capacity; the client reconnects and re-queries everything.
    Overflowed,
    /// The hub and every publisher are gone.
    Closed,
}

impl AppEventHub {
    /// Creates an empty hub with bounded per-client queues.
    pub fn new() -> Self {
        Self {
            subscribers: Arc::default(),
        }
    }

    /// Returns the publisher injected into backend-owned actors.
    pub(crate) fn publisher(&self) -> AppEventPublisher {
        AppEventPublisher {
            subscribers: self.subscribers.clone(),
        }
    }

    /// Returns an application stream that begins with Ready and receives future events.
    pub fn subscribe(&self) -> SessionEventStream<AppEvent> {
        let cancellation = CancellationToken::new();
        let queue = Arc::new(ClientQueue::default());
        queue.push(AppEvent::Ready);
        self.subscribers
            .queues
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::downgrade(&queue));
        let (stream_sender, stream_receiver) = mpsc::channel(APP_EVENT_STREAM_CAPACITY);
        let forward_cancellation = cancellation.clone();
        let worker = tokio::spawn(forward_events(queue, stream_sender, forward_cancellation));

        SessionEventStream::with_cleanup(stream_receiver, move || async move {
            cancellation.cancel();
            worker
                .await
                .map_err(|error| BackendError::internal("application event cleanup failed", error))
        })
    }

    /// Counts the clients whose streams still take events.
    #[cfg(test)]
    fn subscriber_count(&self) -> usize {
        self.subscribers.live().len()
    }
}

impl Default for AppEventHub {
    fn default() -> Self {
        Self::new()
    }
}

impl Subscribers {
    /// Returns the queues of the clients still subscribed, forgetting those that went away.
    fn live(&self) -> Vec<Arc<ClientQueue>> {
        let mut queues = self.queues.lock().unwrap_or_else(PoisonError::into_inner);
        let live: Vec<_> = queues.iter().filter_map(Weak::upgrade).collect();
        queues.retain(|queue| queue.strong_count() > 0);
        live
    }
}

impl Drop for Subscribers {
    /// Ends every stream once nothing can publish to it any more.
    fn drop(&mut self) {
        for queue in self.live() {
            queue.end(StreamEnd::Closed);
        }
    }
}

impl ClientQueue {
    /// Queues one event for this client, merging an invalidation that is already waiting.
    fn push(&self, event: AppEvent) {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.end.is_some() {
                return;
            }
            // The waiting one is delivered after this change too, so it stands for both.
            if matches!(
                event,
                AppEvent::AgentModelsInvalidated { .. } | AppEvent::SessionOptionsChanged { .. }
            ) && state.events.contains(&event)
            {
                return;
            }
            if state.events.len() >= APP_EVENT_QUEUE_CAPACITY {
                ora_debug!("application event stream queue overflowed");
                state.end = Some(StreamEnd::Overflowed);
            } else {
                state.events.push_back(event);
            }
        }
        self.changed.notify_one();
    }

    /// Stops queueing for this client; what is already queued is still delivered.
    fn end(&self, end: StreamEnd) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .end
            .get_or_insert(end);
        self.changed.notify_one();
    }

    /// Takes everything queued, and why the stream ends after it, if it does.
    fn take(&self) -> (VecDeque<AppEvent>, Option<StreamEnd>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        (std::mem::take(&mut state.events), state.end)
    }
}

impl AppEventPublisher {
    /// Publishes one invalidation without blocking the actor that changed durable state.
    pub(crate) fn try_publish(&self, event: AppEvent) {
        let queues = self.subscribers.live();
        if queues.is_empty() {
            ora_debug!("application event dropped because no client is subscribed");
            return;
        }
        for queue in queues {
            queue.push(event.clone());
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

/// Hands one client's queued events to its stream, waiting for the client so publishers never do.
async fn forward_events(
    queue: Arc<ClientQueue>,
    sender: mpsc::Sender<Result<AppEvent, RuntimeError>>,
    cancellation: CancellationToken,
) {
    loop {
        // Registered before looking, so an event queued meanwhile still wakes this loop.
        let changed = queue.changed.notified();
        let (events, end) = queue.take();
        for event in events {
            tokio::select! {
                _ = cancellation.cancelled() => return,
                sent = sender.send(Ok(event)) => if sent.is_err() { return },
            }
        }
        match end {
            // The client's view may be stale now; ending makes it reconnect and re-query.
            Some(StreamEnd::Overflowed) => return,
            Some(StreamEnd::Closed) => {
                let _ = sender.try_send(Err(
                    stream_failure("application event hub was closed").into()
                ));
                return;
            }
            None => {}
        }
        tokio::select! {
            _ = cancellation.cancelled() => return,
            () = changed => {}
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
    /// A burst of invalidations larger than a client's queue, as a batch of Skill changes each told
    /// to every agent, is merged while the client is not reading: the stream stays open and each
    /// agent is told once.
    #[tokio::test]
    async fn merges_agent_invalidations_waiting_for_a_client() {
        let hub = AppEventHub::new();
        let mut stream = hub.subscribe();
        let agents = ["official/first", "official/second", "official/third"];
        let publisher = hub.publisher();
        for _ in 0..100 {
            for agent in agents {
                publisher.try_publish(AppEvent::AgentModelsInvalidated {
                    agent_ref: agent.to_string(),
                });
            }
        }
        let title = AppEvent::SessionTitleUpdated {
            session_id: "after".to_string(),
        };
        publisher.try_publish(title.clone());

        let mut received = Vec::new();
        for _ in 0..1 + agents.len() + 1 {
            received.push(stream.recv().await.unwrap().unwrap());
        }
        let mut expected = vec![AppEvent::Ready];
        expected.extend(agents.map(|agent| AppEvent::AgentModelsInvalidated {
            agent_ref: agent.to_string(),
        }));
        expected.push(title);
        assert_eq!(received, expected);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.recv())
                .await
                .is_err(),
            "nothing more, and the stream is still open"
        );
    }

    /// A burst of option announcements for a session, as an attach replay takes frame after frame,
    /// is merged while the client is not reading: the session is announced once and the stream stays
    /// open.
    #[tokio::test]
    async fn merges_option_announcements_waiting_for_a_client() {
        let hub = AppEventHub::new();
        let mut stream = hub.subscribe();
        let publisher = hub.publisher();
        let sessions = ["first", "second"];
        for _ in 0..100 {
            for session in sessions {
                publisher.try_publish(AppEvent::SessionOptionsChanged {
                    session_id: session.to_string(),
                });
            }
        }

        let mut received = Vec::new();
        for _ in 0..1 + sessions.len() {
            received.push(stream.recv().await.unwrap().unwrap());
        }
        let mut expected = vec![AppEvent::Ready];
        expected.extend(sessions.map(|session| AppEvent::SessionOptionsChanged {
            session_id: session.to_string(),
        }));
        assert_eq!(received, expected);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.recv())
                .await
                .is_err(),
            "nothing more, and the stream is still open"
        );
    }

    /// More distinct events than a client's queue holds still end its stream, so it reconnects and
    /// re-queries instead of missing some.
    #[tokio::test]
    async fn ends_a_stream_whose_client_falls_too_far_behind() {
        let hub = AppEventHub::new();
        let mut stream = hub.subscribe();
        for session in 0..=APP_EVENT_QUEUE_CAPACITY {
            hub.publisher().try_publish(AppEvent::SessionTitleUpdated {
                session_id: session.to_string(),
            });
        }
        let mut received = 0;
        while let Some(event) = tokio::time::timeout(Duration::from_secs(5), stream.recv())
            .await
            .expect("the stream ends")
        {
            event.unwrap();
            received += 1;
        }
        assert_eq!(received, APP_EVENT_QUEUE_CAPACITY);
    }

    /// Confirmation releases this subscriber while leaving independent subscriptions usable.
    #[tokio::test]
    async fn cancellation_confirms_unsubscription_and_preserves_other_streams() {
        let hub = AppEventHub::new();
        let mut first = hub.subscribe();
        let mut second = hub.subscribe();
        assert_eq!(hub.subscriber_count(), 2);
        first
            .cancel_and_wait()
            .await
            .expect("confirmed unsubscribe");
        first.cancel_and_wait().await.expect("repeat confirmation");
        assert_eq!(hub.subscriber_count(), 1);
        assert_eq!(second.recv().await.unwrap().unwrap(), AppEvent::Ready);
        let event = AppEvent::SessionTitleUpdated {
            session_id: "remaining".to_string(),
        };
        hub.publisher().try_publish(event.clone());
        assert_eq!(second.recv().await.unwrap().unwrap(), event);
        second.cancel_and_wait().await.expect("final unsubscribe");
        assert_eq!(hub.subscriber_count(), 0);
    }
}
