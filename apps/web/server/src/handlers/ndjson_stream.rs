use crate::error::{DeferredCompletion, current_lifecycle};
use axum::body::{Body, Bytes};
use axum::http::{HeaderValue, Response, header};
use futures_util::stream;
use ora_backend::{BackendError, RequestLifecycle, SessionEventStream};
use ora_contracts::{ContractError, EmptyErrorParams, PublicError};
use serde::Serialize;
use std::convert::Infallible;
use std::future::Future;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamFrame<Event> {
    Data { data: Event },
    Error { error: ContractError },
    End,
}

/// Supplies one NDJSON event stream that can wait or inspect a buffered item.
pub(crate) trait NdjsonSource: Send + 'static {
    type Event: Serialize + Send + 'static;

    /// Waits for the next event, terminal error, or disconnection.
    fn recv(&mut self) -> impl Future<Output = Option<Result<Self::Event, BackendError>>> + Send;

    /// Returns a buffered item without waiting. `None` means empty or disconnected.
    fn try_recv(&mut self) -> Option<Result<Self::Event, BackendError>>;
}

impl<Event> NdjsonSource for SessionEventStream<Event>
where
    Event: Serialize + Send + 'static,
{
    type Event = Event;

    fn recv(&mut self) -> impl Future<Output = Option<Result<Self::Event, BackendError>>> + Send {
        SessionEventStream::recv(self)
    }

    fn try_recv(&mut self) -> Option<Result<Self::Event, BackendError>> {
        SessionEventStream::try_recv(self)
    }
}

impl<Event> NdjsonSource for mpsc::Receiver<Result<Event, BackendError>>
where
    Event: Serialize + Send + 'static,
{
    type Event = Event;

    fn recv(&mut self) -> impl Future<Output = Option<Result<Self::Event, BackendError>>> + Send {
        mpsc::Receiver::recv(self)
    }

    fn try_recv(&mut self) -> Option<Result<Self::Event, BackendError>> {
        mpsc::Receiver::try_recv(self).ok()
    }
}

/// Converts one event source into ordered, atomic NDJSON transport frames.
pub(crate) fn stream_response<S>(source: S, shutdown: CancellationToken) -> Response<Body>
where
    S: NdjsonSource,
{
    let lifecycle = current_lifecycle();
    let body_stream = stream::unfold(
        (source, false, lifecycle, shutdown),
        |(mut source, ended, lifecycle, shutdown)| async move {
            if ended {
                return None;
            }
            let (frame, next_ended) = tokio::select! {
                biased;
                _ = shutdown.cancelled() => frame_after_shutdown(&mut source, &lifecycle),
                event = source.recv() => encode_event(event, &lifecycle),
            };
            let mut bytes = serde_json::to_vec(&frame).unwrap_or_else(|source| {
                let error = BackendError::internal("failed to encode stream frame", source);
                lifecycle.complete_failure(&error);
                serde_json::to_vec(&StreamFrame::<S::Event>::Error {
                    error: ContractError {
                        error: PublicError::InternalError(EmptyErrorParams {}),
                        request_id: lifecycle.request_id(),
                    },
                })
                .unwrap_or_default()
            });
            bytes.push(b'\n');
            Some((
                Ok::<Bytes, Infallible>(Bytes::from(bytes)),
                (source, next_ended, lifecycle, shutdown),
            ))
        },
    );
    let mut response = Response::new(Body::from_stream(body_stream));
    response.extensions_mut().insert(DeferredCompletion);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    response
}

/// Drains already-queued items so a buffered terminal error is not replaced by a successful `end`.
///
/// `select!` treats `cancelled()` and a ready `recv()` as equally ready unless `biased` is set.
/// Shutdown is the first arm so a cancelled token always wins; `try_recv` then inspects the
/// buffer for a terminal error instead of assuming the stream completed cleanly. Buffered data is
/// discarded because waiting for future events would pin the process. A buffered error still
/// fails the request.
fn frame_after_shutdown<S: NdjsonSource>(
    source: &mut S,
    lifecycle: &RequestLifecycle,
) -> (StreamFrame<S::Event>, bool) {
    loop {
        match source.try_recv() {
            Some(Err(error)) => return encode_event(Some(Err(error)), lifecycle),
            Some(Ok(_)) => {}
            None => return encode_event(None, lifecycle),
        }
    }
}

/// Maps one source outcome onto the private NDJSON transport frame and completion flag.
fn encode_event<Event>(
    event: Option<Result<Event, BackendError>>,
    lifecycle: &RequestLifecycle,
) -> (StreamFrame<Event>, bool) {
    match event {
        Some(Ok(event)) => (StreamFrame::Data { data: event }, false),
        Some(Err(error)) => {
            lifecycle.complete_failure(&error);
            (
                StreamFrame::Error {
                    error: error.contract_error(lifecycle.request_id()),
                },
                true,
            )
        }
        None => {
            lifecycle.complete_success();
            (StreamFrame::End, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::stream_response;
    use futures_util::StreamExt;
    use ora_backend::BackendError;
    use ora_contracts::WorkspaceFileEventBatch;
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    /// Verifies a live workspace watch still ends when process shutdown is requested.
    #[tokio::test]
    async fn workspace_watch_stream_ends_when_shutdown_is_requested() {
        let (sender, receiver) =
            tokio::sync::mpsc::channel::<Result<WorkspaceFileEventBatch, BackendError>>(1);
        let shutdown = CancellationToken::new();
        let response = stream_response(receiver, shutdown.clone());
        let mut body = response.into_body().into_data_stream();

        assert!(
            tokio::time::timeout(Duration::from_millis(50), body.next())
                .await
                .is_err(),
            "a live watcher must stay open until shutdown or a filesystem event"
        );

        sender
            .send(Ok(WorkspaceFileEventBatch {
                changes: Vec::new(),
            }))
            .await
            .unwrap_or_else(|error| panic!("send watch batch: {error}"));
        let data = next_frame(&mut body).await;
        assert_eq!(
            data,
            json!({
                "type": "data",
                "data": { "changes": [] }
            })
        );

        shutdown.cancel();
        let end = next_frame(&mut body).await;
        assert_eq!(end, json!({ "type": "end" }));
        let finished = tokio::time::timeout(Duration::from_millis(200), body.next()).await;
        assert!(
            matches!(finished, Ok(None)),
            "the body must complete after the end frame, got {finished:?}"
        );
        drop(sender);
    }

    /// Verifies shutdown cannot replace a buffered terminal error with a successful end frame.
    #[tokio::test]
    async fn shutdown_emits_a_buffered_error_instead_of_end() {
        for _ in 0..32 {
            let (sender, receiver) =
                tokio::sync::mpsc::channel::<Result<WorkspaceFileEventBatch, BackendError>>(2);
            sender
                .try_send(Ok(WorkspaceFileEventBatch {
                    changes: Vec::new(),
                }))
                .expect("data is queued");
            sender
                .try_send(Err(BackendError::internal(
                    "watcher failed",
                    std::io::Error::other("closed"),
                )))
                .expect("error is queued");
            let shutdown = CancellationToken::new();
            shutdown.cancel();
            let response = stream_response(receiver, shutdown);
            let mut body = response.into_body().into_data_stream();
            let frame = next_frame(&mut body).await;
            let request_id = frame["error"]["requestId"].clone();
            assert_eq!(
                frame,
                json!({
                    "type": "error",
                    "error": {
                        "code": "internal_error",
                        "params": {},
                        "requestId": request_id,
                    }
                })
            );
        }
    }

    /// Verifies shutdown still ends after discarding buffered data that is not a terminal error.
    #[tokio::test]
    async fn shutdown_discards_buffered_data_and_ends() {
        let (sender, receiver) =
            tokio::sync::mpsc::channel::<Result<WorkspaceFileEventBatch, BackendError>>(1);
        sender
            .try_send(Ok(WorkspaceFileEventBatch {
                changes: Vec::new(),
            }))
            .expect("data is queued");
        let shutdown = CancellationToken::new();
        shutdown.cancel();
        let response = stream_response(receiver, shutdown);
        let mut body = response.into_body().into_data_stream();
        let end = next_frame(&mut body).await;
        assert_eq!(end, json!({ "type": "end" }));
        drop(sender);
    }

    /// Reads one NDJSON transport frame from a watch body.
    async fn next_frame<E>(
        body: &mut (impl StreamExt<Item = Result<axum::body::Bytes, E>> + Unpin),
    ) -> Value
    where
        E: std::fmt::Debug,
    {
        let chunk = tokio::time::timeout(Duration::from_secs(1), body.next())
            .await
            .unwrap_or_else(|_| panic!("watch frame timed out"))
            .unwrap_or_else(|| panic!("watch frame is missing"))
            .unwrap_or_else(|error| panic!("watch frame: {error:?}"));
        serde_json::from_slice(chunk.trim_ascii())
            .unwrap_or_else(|error| panic!("watch frame json: {error}"))
    }
}
