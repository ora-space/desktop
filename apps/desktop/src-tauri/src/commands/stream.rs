//! Shared stream startup, cancellation, and forwarding; domain routing is generated.

use super::stream_routes::{self, StreamOperation};
use crate::stream_forwarding::{forward_contract_stream, forward_workspace_watch};
use crate::stream_registry::StreamRegistration;
use crate::{error::CommandError, state::DesktopState};
use ora_backend::{
    BackendError, ErrorClassification, RequestLifecycle, SessionEventStream, UuidRequestIdGenerator,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use std::future::Future;
use tauri::{State, ipc::Channel};
use tokio_util::sync::CancellationToken;

/// Owns a request and registration until startup transfers them to a forwarding task.
pub(super) struct StreamStart {
    registration: StreamRegistration,
    channel: Channel<serde_json::Value>,
    lifecycle: RequestLifecycle,
}

/// Distinguishes cancelled startup from a resource ready to transfer to its forwarding task.
enum Startup<T> {
    Cancelled,
    Ready(T),
}

impl StreamStart {
    /// Starts an ordered backend event source and transfers exactly one lifecycle to forwarding.
    pub(super) async fn events<T: serde::Serialize + Send + 'static>(
        self,
        source: impl Future<Output = Result<SessionEventStream<T>, BackendError>>,
    ) -> Result<(), CommandError> {
        match settle_startup(source, self.registration.cancellation(), &self.lifecycle).await? {
            Startup::Cancelled => {}
            Startup::Ready(stream) => {
                tauri::async_runtime::spawn(forward_contract_stream(
                    stream,
                    self.registration,
                    self.channel,
                    self.lifecycle,
                ));
            }
        }
        Ok(())
    }

    /// Starts a native event source while using the same cancellation and request ownership rules.
    pub(super) async fn watch(
        self,
        source: impl Future<Output = Result<ora_fs::WorkspaceWatcher, BackendError>>,
    ) -> Result<(), CommandError> {
        match settle_startup(source, self.registration.cancellation(), &self.lifecycle).await? {
            Startup::Cancelled => {}
            Startup::Ready(watcher) => {
                tauri::async_runtime::spawn(forward_workspace_watch(
                    watcher,
                    self.registration,
                    self.channel,
                    self.lifecycle,
                ));
            }
        }
        Ok(())
    }
}

/// Lets started domain work settle before dropping its resource; arbitrary startup futures may
/// already have committed actor side effects and are not safe to abandon midway through creation.
async fn settle_startup<T>(
    source: impl Future<Output = Result<T, BackendError>>,
    cancellation: &CancellationToken,
    lifecycle: &RequestLifecycle,
) -> Result<Startup<T>, CommandError> {
    if cancellation.is_cancelled() {
        lifecycle.complete_cancellation();
        return Ok(Startup::Cancelled);
    }
    let resource = source
        .await
        .map_err(|error| CommandError::from_backend_with_lifecycle(error, lifecycle))?;
    if cancellation.is_cancelled() {
        drop(resource);
        lifecycle.complete_cancellation();
        Ok(Startup::Cancelled)
    } else {
        Ok(Startup::Ready(resource))
    }
}

/// Validates a typed request and claims its id before any domain startup work can run.
#[tauri::command]
pub async fn stream_contract(
    state: State<'_, DesktopState>,
    operation_name: String,
    request: serde_json::Value,
    stream_call_id: String,
    on_event: Channel<serde_json::Value>,
) -> Result<(), CommandError> {
    let lifecycle = RequestLifecycle::start(
        format!("stream_contract:{operation_name}"),
        &UuidRequestIdGenerator,
    );
    let operation = serde_json::from_value::<StreamOperation>(serde_json::json!({
        "operationName": operation_name,
        "request": request,
    }))
    .map_err(|error| {
        CommandError::from_backend_with_lifecycle(
            BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                format!("invalid stream request: {error}"),
            ),
            &lifecycle,
        )
    })?;
    let registration = state
        .streams
        .register(stream_call_id)
        .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
    stream_routes::start(
        state,
        operation,
        StreamStart {
            registration,
            channel: on_event,
            lifecycle,
        },
    )
    .await
}

/// Cancels starting or running streams without prematurely releasing their private ids.
#[tauri::command]
pub async fn cancel_contract_stream(
    state: State<'_, DesktopState>,
    stream_call_id: String,
) -> Result<(), CommandError> {
    let lifecycle = RequestLifecycle::start("cancel_contract_stream", &UuidRequestIdGenerator);
    let request_span =
        ora_logging::span_with_request_id("tauri_command", &lifecycle.request_id().to_string());
    request_span.in_scope(|| {
        state
            .streams
            .cancel(&stream_call_id)
            .map_err(|error| CommandError::from_backend_with_lifecycle(error, &lifecycle))?;
        lifecycle.complete_success();
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::super::stream_routes::StreamOperation;
    use super::{Startup, settle_startup};
    use ora_backend::{BackendError, RequestLifecycle, UuidRequestIdGenerator};
    use ora_logging::with_trace_logging;
    use pretty_assertions::assert_eq;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio_util::sync::CancellationToken;

    struct Resource(Arc<AtomicUsize>);

    impl Drop for Resource {
        /// Records release of a resource that completed creation after its caller cancelled.
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Pre-cancelled work is never polled; cancellation during creation waits for safe cleanup.
    #[test]
    fn cancellation_before_and_during_creation_releases_resources() {
        with_trace_logging(|| {
            tauri::async_runtime::block_on(async {
                let token = CancellationToken::new();
                token.cancel();
                let lifecycle = RequestLifecycle::start("pre_cancel", &UuidRequestIdGenerator);
                let polls = AtomicUsize::new(0);
                let result = settle_startup(
                    async {
                        polls.fetch_add(1, Ordering::SeqCst);
                        Ok::<(), BackendError>(())
                    },
                    &token,
                    &lifecycle,
                )
                .await
                .expect("pre-cancel succeeds");
                assert!(matches!(result, Startup::Cancelled));
                assert_eq!(polls.load(Ordering::SeqCst), 0);

                let token = CancellationToken::new();
                let drops = Arc::new(AtomicUsize::new(0));
                let lifecycle =
                    RequestLifecycle::start("cancel_during_creation", &UuidRequestIdGenerator);
                let result = settle_startup(
                    async {
                        token.cancel();
                        Ok(Resource(drops.clone()))
                    },
                    &token,
                    &lifecycle,
                )
                .await
                .expect("creation settles");
                assert!(matches!(result, Startup::Cancelled));
                assert_eq!(drops.load(Ordering::SeqCst), 1);
            });
        });
    }

    /// A creation failure remains a correlated failure even when cancellation races with it.
    #[test]
    fn startup_failure_preserves_the_request_id() {
        with_trace_logging(|| {
            tauri::async_runtime::block_on(async {
                let token = CancellationToken::new();
                let lifecycle = RequestLifecycle::start("failed_start", &UuidRequestIdGenerator);
                let result = settle_startup(
                    async {
                        token.cancel();
                        Err::<(), _>(BackendError::internal(
                            "fixture startup",
                            std::io::Error::other("failed"),
                        ))
                    },
                    &token,
                    &lifecycle,
                )
                .await;
                let Err(error) = result else {
                    panic!("startup must fail");
                };
                assert_eq!(
                    serde_json::to_value(error).expect("serialize public error"),
                    serde_json::json!({
                        "code": "internal_error",
                        "params": {},
                        "requestId": lifecycle.request_id(),
                    })
                );
            });
        });
    }

    /// The generated decoder accepts only declared stream operations and their actual DTO shape.
    #[test]
    fn generated_stream_requests_reject_unknown_operations_and_bad_payloads() {
        assert!(matches!(
            serde_json::from_value::<StreamOperation>(
                serde_json::json!({"operationName": "watchProject", "request": {"projectId": "fixture"}})
            ),
            Ok(StreamOperation::WatchProject(_))
        ));
        assert!(
            serde_json::from_value::<StreamOperation>(
                serde_json::json!({"operationName": "notDeclared", "request": {}})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<StreamOperation>(
                serde_json::json!({"operationName": "watchProject", "request": {}})
            )
            .is_err()
        );
    }
}
