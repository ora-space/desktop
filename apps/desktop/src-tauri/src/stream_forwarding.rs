//! Bridges backend event streams onto Tauri IPC Channels.
//!
//! Every stream started at the command seam is deferred: the command returns before any event
//! exists, so the forwarding task owns the request's `RequestLifecycle` and is solely responsible
//! for emitting its single completion event. Each way a stream can end — caller cancellation, a
//! terminal backend error, natural exhaustion, or the frontend Channel disappearing — must claim
//! that completion, otherwise the request leaves no closing record in the logs.
//!
//! Terminal frames claim their completion from the backend outcome *before* being sent, so a
//! Channel that dies while the last frame is in flight keeps the real backend result in the log
//! rather than rewriting it as a cancellation. The recorded outcome therefore describes how the
//! backend stream ended, not whether the webview observed its final frame.

use crate::stream_registry::StreamRegistration;
use crate::workspace_files::workspace_file_backend_error;
use ora_backend::{BackendError, RequestLifecycle};
use ora_contracts::{WorkspaceFileChange, WorkspaceFileEventBatch};
use serde::Serialize;
use std::time::Duration;
use tauri::ipc::Channel;

/// Forwards ordered data/error/end frames and drops the backend stream on channel failure.
pub(crate) async fn forward_contract_stream<Event>(
    mut stream: ora_backend::SessionEventStream<Event>,
    registration: StreamRegistration,
    on_event: Channel<serde_json::Value>,
    lifecycle: RequestLifecycle,
) where
    Event: Serialize + Send + 'static,
{
    let cancellation = registration.cancellation().clone();
    loop {
        tokio::select! {
            () = cancellation.cancelled() => {
                lifecycle.complete_cancellation();
                break;
            },
            event = stream.recv() => {
                let is_terminal = matches!(&event, Some(Err(_)) | None);
                let frame = match event {
                    Some(Ok(data)) => serde_json::json!({ "type": "data", "data": data }),
                    Some(Err(error)) => {
                        lifecycle.complete_failure(&error);
                        serde_json::json!({
                            "type": "error",
                            "error": error.contract_error(lifecycle.request_id()),
                        })
                    },
                    None => {
                        lifecycle.complete_success();
                        serde_json::json!({ "type": "end" })
                    },
                };
                if on_event.send(frame).is_err() {
                    // The frontend Channel is gone, so no terminal frame can ever be delivered.
                    // Record it as a caller-side teardown rather than a backend failure; the
                    // exactly-once claim makes this a no-op when a terminal frame already
                    // completed the request just above.
                    lifecycle.complete_cancellation();
                    break;
                }
                if is_terminal {
                    break;
                }
            }
        }
    }
    drop(registration);
}

/// Forwards debounced native workspace changes until the Desktop stream is cancelled.
pub(crate) async fn forward_workspace_watch(
    watcher: ora_fs::WorkspaceWatcher,
    registration: StreamRegistration,
    on_event: Channel<serde_json::Value>,
    lifecycle: RequestLifecycle,
) {
    let cancellation = registration.cancellation().clone();
    let watch_cancellation = cancellation.clone();
    let terminal_channel = on_event.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        while !watch_cancellation.is_cancelled() {
            match watcher.receive_batch(Duration::from_millis(100)) {
                Ok(Some(changes)) if !changes.is_empty() => {
                    let data = WorkspaceFileEventBatch {
                        changes: changes.into_iter().map(to_contract_change).collect(),
                    };
                    if on_event
                        .send(serde_json::json!({ "type": "data", "data": data }))
                        .is_err()
                    {
                        return Err(WatchStop::ChannelClosed);
                    }
                }
                Ok(Some(_)) | Ok(None) => {}
                Err(error) => return Err(WatchStop::Watcher(error)),
            }
        }
        Ok(())
    })
    .await;

    if cancellation.is_cancelled() {
        lifecycle.complete_cancellation();
    } else {
        match result {
            Ok(Ok(())) => {
                lifecycle.complete_success();
                let _ = terminal_channel.send(serde_json::json!({ "type": "end" }));
            }
            // A closed Channel has no receiver left to inform, so this path only records the
            // completion that the disconnected caller can no longer observe.
            Ok(Err(WatchStop::ChannelClosed)) => lifecycle.complete_cancellation(),
            Ok(Err(WatchStop::Watcher(error))) => {
                let backend_error = workspace_file_backend_error(error);
                lifecycle.complete_failure(&backend_error);
                let _ = terminal_channel.send(serde_json::json!({
                    "type": "error",
                    "error": backend_error.contract_error(lifecycle.request_id()),
                }));
            }
            Err(error) => {
                let backend_error =
                    BackendError::internal("Desktop workspace watcher failed", error);
                lifecycle.complete_failure(&backend_error);
                let _ = terminal_channel.send(serde_json::json!({
                    "type": "error",
                    "error": backend_error.contract_error(lifecycle.request_id()),
                }));
            }
        }
    }
    drop(registration);
}

/// Distinguishes the two reasons the blocking watch loop stops before cancellation.
enum WatchStop {
    ChannelClosed,
    Watcher(ora_fs::WorkspaceFileSystemError),
}

/// Converts native watcher events to the shared file-change contract.
fn to_contract_change(change: ora_fs::WorkspaceChange) -> WorkspaceFileChange {
    match change.kind {
        ora_fs::WorkspaceChangeKind::Created => WorkspaceFileChange::Created { path: change.path },
        ora_fs::WorkspaceChangeKind::Modified => {
            WorkspaceFileChange::Modified { path: change.path }
        }
        ora_fs::WorkspaceChangeKind::Removed => WorkspaceFileChange::Removed { path: change.path },
        ora_fs::WorkspaceChangeKind::Renamed { from } => WorkspaceFileChange::Renamed {
            from,
            path: change.path,
        },
        ora_fs::WorkspaceChangeKind::RescanRequired => WorkspaceFileChange::RescanRequired,
    }
}

#[cfg(test)]
mod tests {
    use super::{forward_contract_stream, forward_workspace_watch};
    use crate::stream_registry::StreamRegistry;
    use ora_backend::{AppEventHub, RequestLifecycle, UuidRequestIdGenerator};
    use ora_logging::with_recorded_trace_logging;
    use pretty_assertions::assert_eq;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tauri::ipc::Channel;
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, Layer};

    const STREAM_CALL_ID: &str = "test-stream-call-id";

    /// Bounds the wait for a native filesystem event so a silent platform fails instead of hanging.
    const NATIVE_EVENT_TIMEOUT: Duration = Duration::from_secs(30);

    /// A stream whose frontend Channel disconnects must still record one completion event.
    ///
    /// The disconnect happens on a non-terminal data frame, which is the case that previously
    /// broke out of the forwarding loop without claiming the lifecycle at all.
    #[test]
    fn channel_disconnect_on_a_data_frame_completes_the_request_as_cancelled() {
        let recorder = OutcomeRecorder::default();
        let registry = StreamRegistry::default();
        let registration = registry
            .register(STREAM_CALL_ID.to_string())
            .expect("register stream");
        let send_attempts = Arc::new(AtomicUsize::new(0));

        with_recorded_trace_logging(recorder.layer(), || {
            runtime().block_on(async {
                // `subscribe` seeds the stream with `AppEvent::Ready`, so the first frame the
                // loop forwards is a non-terminal data frame.
                let stream = AppEventHub::new().subscribe();
                forward_contract_stream(
                    stream,
                    registration,
                    disconnected_channel(send_attempts.clone()),
                    RequestLifecycle::start("test_stream", &UuidRequestIdGenerator),
                )
                .await;
            });
        });

        assert_eq!(send_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(recorder.outcomes(), vec!["cancelled".to_string()]);
        assert!(registry.register(STREAM_CALL_ID.to_string()).is_ok());
    }

    /// Cancelling a live stream records exactly one cancellation and releases its registration.
    ///
    /// The Channel here stays connected, so the loop leaves through the cancellation branch
    /// rather than the disconnect branch even when `select!` forwards the seeded event first.
    #[test]
    fn cancellation_completes_the_request_once_and_releases_the_registration() {
        let recorder = OutcomeRecorder::default();
        let registry = StreamRegistry::default();
        let registration = registry
            .register(STREAM_CALL_ID.to_string())
            .expect("register stream");
        registry.cancel(STREAM_CALL_ID).expect("cancel stream");

        with_recorded_trace_logging(recorder.layer(), || {
            runtime().block_on(async {
                let stream = AppEventHub::new().subscribe();
                forward_contract_stream(
                    stream,
                    registration,
                    connected_channel(),
                    RequestLifecycle::start("test_stream", &UuidRequestIdGenerator),
                )
                .await;
            });
        });

        assert_eq!(recorder.outcomes(), vec!["cancelled".to_string()]);
        assert!(registry.register(STREAM_CALL_ID.to_string()).is_ok());
    }

    /// A watch stream whose frontend Channel disconnects completes as cancelled, not success.
    ///
    /// The blocking watch loop returns `Ok(())` on both a clean stop and a dead Channel, so this
    /// pins the outcome that distinguishes them and would silently regress to `success` if the
    /// disconnect were ever folded back into the normal exit.
    #[test]
    fn watch_channel_disconnect_completes_the_request_as_cancelled() {
        let recorder = OutcomeRecorder::default();
        let registry = StreamRegistry::default();
        let registration = registry
            .register(STREAM_CALL_ID.to_string())
            .expect("register stream");
        let workspace = tempfile::TempDir::new().unwrap();
        let watcher = ora_fs::WorkspaceWatcher::start(workspace.path()).unwrap();
        // The watcher is already running, so this change is queued before forwarding starts and
        // the loop's first batch is the data frame whose send must fail.
        std::fs::write(workspace.path().join("watched.txt"), "changed").unwrap();
        let send_attempts = Arc::new(AtomicUsize::new(0));
        let timeout_guard = registration.cancellation().clone();
        std::thread::spawn(move || {
            std::thread::sleep(NATIVE_EVENT_TIMEOUT);
            timeout_guard.cancel();
        });

        with_recorded_trace_logging(recorder.layer(), || {
            runtime().block_on(forward_workspace_watch(
                watcher,
                registration,
                disconnected_channel(send_attempts.clone()),
                RequestLifecycle::start("test_watch_stream", &UuidRequestIdGenerator),
            ));
        });

        // A zero count means the safety net cancelled the stream before any native event arrived,
        // which would make the expected outcome pass for the wrong reason.
        assert_eq!(send_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(recorder.outcomes(), vec!["cancelled".to_string()]);
        assert!(registry.register(STREAM_CALL_ID.to_string()).is_ok());
    }

    /// Builds the current-thread runtime that keeps the scoped subscriber on the test thread.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// Builds a Channel that behaves like a webview whose listener has already gone away.
    fn disconnected_channel(send_attempts: Arc<AtomicUsize>) -> Channel<serde_json::Value> {
        Channel::new(move |_body| {
            send_attempts.fetch_add(1, Ordering::SeqCst);
            Err(std::io::Error::other("channel closed").into())
        })
    }

    /// Builds a Channel that accepts every frame, like a webview still listening.
    fn connected_channel() -> Channel<serde_json::Value> {
        Channel::new(|_body| Ok(()))
    }

    /// Captures the `outcome` field of lifecycle completion events without global subscriber state.
    #[derive(Clone, Debug, Default)]
    struct OutcomeRecorder {
        outcomes: Arc<Mutex<Vec<String>>>,
    }

    impl OutcomeRecorder {
        /// Builds the scoped subscriber layer used by one test.
        fn layer(&self) -> OutcomeRecordingLayer {
            OutcomeRecordingLayer {
                outcomes: self.outcomes.clone(),
            }
        }

        /// Returns captured completion outcomes in emission order.
        fn outcomes(&self) -> Vec<String> {
            self.outcomes.lock().unwrap().clone()
        }
    }

    /// Records completion outcomes while leaving production formatting untouched.
    #[derive(Clone, Debug)]
    struct OutcomeRecordingLayer {
        outcomes: Arc<Mutex<Vec<String>>>,
    }

    impl<S> Layer<S> for OutcomeRecordingLayer
    where
        S: tracing::Subscriber,
    {
        /// Collects the `outcome` field, which only lifecycle completion events carry.
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
            let mut visitor = OutcomeVisitor { outcome: None };
            event.record(&mut visitor);
            if let Some(outcome) = visitor.outcome {
                self.outcomes.lock().unwrap().push(outcome);
            }
        }
    }

    /// Extracts the single `outcome` field from one recorded event.
    struct OutcomeVisitor {
        outcome: Option<String>,
    }

    impl Visit for OutcomeVisitor {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "outcome" {
                self.outcome = Some(value.to_string());
            }
        }

        fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
    }
}
