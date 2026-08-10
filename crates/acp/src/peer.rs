use super::pending::{
    PendingRequest, PendingRequests, PendingResponse, ResponseRequest, lock_pending,
};
use super::trace::{SessionTraceRegistration, SessionTraceRegistry};
use agent_client_protocol_schema::v1::CLIENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::RequestId;
use agent_client_protocol_schema::v1::RequestPermissionRequest;
use agent_client_protocol_schema::v1::SessionId;
use agent_client_protocol_schema::v1::SessionNotification;
use futures_util::StreamExt;
use ora_logging::ora_trace;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Reports framing, correlation, serialization, and process-pipe failures.
#[derive(Debug, Error)]
pub enum AcpError {
    #[error("ACP stream ended before the pending operation completed")]
    StreamClosed,
    #[error("ACP transport I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("ACP frame is invalid: {0}")]
    InvalidFrame(String),
    #[error("ACP frame exceeds 8 MiB")]
    FrameTooLarge,
    #[error("ACP returned an operation error: {0}")]
    RequestFailed(String),
    #[error("ACP response payload is invalid: {0}")]
    InvalidResponse(String),
}

/// Carries one permission request together with its JSON-RPC correlation id.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRequest {
    pub request_id: RequestId,
    pub request: RequestPermissionRequest,
}

/// Carries a response that terminates one ordered session request.
#[derive(Debug)]
pub struct SessionResponse {
    request_id: RequestId,
    session_id: SessionId,
    response: PendingResponse,
}

/// Retires a direct request when its waiting future is cancelled or times out.
///
/// Direct responses bypass the ordered session-event stream, but they still need a bounded
/// tombstone after cancellation so a late provider response is ignored rather than treated as an
/// unknown correlation id. Keeping this guard inside `request` makes every caller cancellation
/// safe without exposing correlation bookkeeping in the public API.
struct DirectRequestRegistration {
    request_id: RequestId,
    pending: Arc<Mutex<PendingRequests>>,
    unregister_on_drop: bool,
}

impl DirectRequestRegistration {
    /// Stops Drop cleanup after the reader has already consumed the correlation entry.
    fn complete(&mut self) {
        self.unregister_on_drop = false;
    }

    /// Removes a request that failed before a valid response could be produced.
    fn remove(&mut self) {
        lock_pending(&self.pending).remove_active(&self.request_id);
        self.unregister_on_drop = false;
    }
}

impl Drop for DirectRequestRegistration {
    fn drop(&mut self) {
        if self.unregister_on_drop {
            lock_pending(&self.pending).abandon(&self.request_id);
        }
    }
}

impl SessionResponse {
    /// Identifies the provider session that owns this response.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
}

/// Completes a typed session request after its ordered response event is received.
///
/// Dropping an unsettled handle retires its id so a late response can be discarded without
/// masking a genuinely unknown correlation id.
pub struct PendingSessionRequest<Response> {
    request_id: RequestId,
    session_id: SessionId,
    pending: Arc<Mutex<PendingRequests>>,
    unregister_on_drop: bool,
    response: PhantomData<Response>,
}

impl<Response> PendingSessionRequest<Response>
where
    Response: DeserializeOwned,
{
    /// Returns whether this handle owns the terminating response.
    pub fn matches_response(&self, response: &SessionResponse) -> bool {
        response.request_id == self.request_id && response.session_id == self.session_id
    }

    /// Validates response ownership before decoding the typed result.
    pub fn finish(mut self, response: SessionResponse) -> Result<Response, AcpError> {
        if !self.matches_response(&response) {
            // The handle is consumed here; unregister so correlation cannot leak forever.
            // Callers that need to keep waiting must filter with `matches_response` first.
            lock_pending(&self.pending).abandon(&self.request_id);
            self.unregister_on_drop = false;
            return Err(AcpError::InvalidResponse(format!(
                "response {response_id} for session {response_session_id} does not match request {request_id} for session {request_session_id}",
                response_id = response.request_id,
                response_session_id = response.session_id,
                request_id = self.request_id,
                request_session_id = self.session_id,
            )));
        }
        // The reader already removed this id when the response entered the inbound stream.
        self.unregister_on_drop = false;
        match response.response {
            Ok(result) => serde_json::from_value(result)
                .map_err(|error| AcpError::InvalidResponse(error.to_string())),
            Err(error) => Err(AcpError::RequestFailed(error.message)),
        }
    }

    /// Retires the request so its late response can be discarded without masking unknown ids.
    pub fn abandon(mut self) {
        lock_pending(&self.pending).abandon(&self.request_id);
        self.unregister_on_drop = false;
    }
}

impl<Response> Drop for PendingSessionRequest<Response> {
    fn drop(&mut self) {
        if self.unregister_on_drop {
            lock_pending(&self.pending).abandon(&self.request_id);
        }
    }
}

/// Preserves wire order for all events that participate in a session turn.
#[derive(Debug)]
pub enum AcpInboundEvent {
    SessionUpdate(SessionNotification),
    PermissionRequest(PermissionRequest),
    SessionResponse(SessionResponse),
    Fatal(AcpError),
}

/// Sends correlated ACP requests and protocol responses over one serialized writer.
pub struct AcpClient<Writer> {
    writer: Arc<AsyncMutex<Writer>>,
    pending: Arc<Mutex<PendingRequests>>,
    next_request_id: Arc<AtomicI64>,
    trace_sessions: SessionTraceRegistry,
}

impl<Writer> Clone for AcpClient<Writer> {
    fn clone(&self) -> Self {
        Self {
            writer: self.writer.clone(),
            pending: self.pending.clone(),
            next_request_id: self.next_request_id.clone(),
            trace_sessions: self.trace_sessions.clone(),
        }
    }
}

impl<Writer> AcpClient<Writer>
where
    Writer: AsyncWrite + Unpin + Send + 'static,
{
    /// Associates provider traffic with the Ora session identifier used by application logs.
    pub fn register_session_trace(
        &self,
        agent_session_id: &str,
        ora_session_id: &str,
    ) -> SessionTraceRegistration {
        self.trace_sessions
            .register(agent_session_id, ora_session_id)
    }

    /// Sends a typed request and waits for the independently-read correlated response.
    pub async fn request<Request, Response>(
        &self,
        method: &str,
        params: &Request,
    ) -> Result<Response, AcpError>
    where
        Request: Serialize,
        Response: DeserializeOwned,
    {
        let request_id = RequestId::Number(self.next_request_id.fetch_add(1, Ordering::Relaxed));
        let (response_sender, response_receiver) = oneshot::channel();
        lock_pending(&self.pending)
            .insert(request_id.clone(), PendingRequest::Direct(response_sender));
        let mut registration = DirectRequestRegistration {
            request_id: request_id.clone(),
            pending: Arc::clone(&self.pending),
            unregister_on_drop: true,
        };
        let frame = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        if let Err(error) = self.write_frame(&frame).await {
            registration.remove();
            return Err(error);
        }
        let response = response_receiver
            .await
            .map_err(|_| AcpError::StreamClosed)?;
        // The reader removes direct requests before delivering their response through the
        // oneshot, so Drop must not turn a successfully correlated request into a tombstone.
        registration.complete();
        match response {
            Ok(result) => serde_json::from_value(result)
                .map_err(|error| AcpError::InvalidResponse(error.to_string())),
            Err(error) => Err(AcpError::RequestFailed(error.message)),
        }
    }

    /// Starts a session request whose response must remain ordered with session events.
    pub async fn start_session_request<Request, Response>(
        &self,
        session_id: SessionId,
        method: &str,
        params: &Request,
    ) -> Result<PendingSessionRequest<Response>, AcpError>
    where
        Request: Serialize,
        Response: DeserializeOwned,
    {
        let request_id = RequestId::Number(self.next_request_id.fetch_add(1, Ordering::Relaxed));
        lock_pending(&self.pending).insert(
            request_id.clone(),
            PendingRequest::Session {
                session_id: session_id.clone(),
            },
        );
        let frame = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        if let Err(error) = self.write_frame(&frame).await {
            lock_pending(&self.pending).remove_active(&request_id);
            return Err(error);
        }
        Ok(PendingSessionRequest {
            request_id,
            session_id,
            pending: self.pending.clone(),
            unregister_on_drop: true,
            response: PhantomData,
        })
    }

    /// Sends a notification that intentionally has no JSON-RPC response.
    pub async fn notify<Params>(&self, method: &str, params: &Params) -> Result<(), AcpError>
    where
        Params: Serialize,
    {
        self.write_frame(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    /// Responds to an agent-originated permission request with a typed result payload.
    pub async fn respond<ResultBody>(
        &self,
        request_id: &RequestId,
        result: &ResultBody,
    ) -> Result<(), AcpError>
    where
        ResultBody: Serialize,
    {
        self.write_frame(&json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": result,
        }))
        .await
    }

    /// Serializes one complete NDJSON frame so concurrent control writes cannot interleave.
    async fn write_frame(&self, value: &Value) -> Result<(), AcpError> {
        write_frame(&self.writer, &self.trace_sessions, value).await
    }
}

/// Owns the ordered inbound receiver for one ACP connection.
pub struct AcpPeer<Writer> {
    pub client: AcpClient<Writer>,
    inbound: mpsc::UnboundedReceiver<AcpInboundEvent>,
}

impl<Writer> AcpPeer<Writer>
where
    Writer: AsyncWrite + Unpin + Send + 'static,
{
    /// Starts the reader task and delegates session-event flow control to the connection owner.
    pub fn spawn<Reader>(reader: Reader, writer: Writer) -> Self
    where
        Reader: AsyncRead + Unpin + Send + 'static,
    {
        let pending = Arc::new(Mutex::new(PendingRequests::default()));
        let writer = Arc::new(AsyncMutex::new(writer));
        let trace_sessions = SessionTraceRegistry::default();
        // The application router applies bounded queues per provider session. Bounding this
        // connection-wide handoff would let one noisy session terminate every other session.
        let (inbound_sender, inbound) = mpsc::unbounded_channel();
        tokio::spawn(read_frames(
            reader,
            writer.clone(),
            pending.clone(),
            trace_sessions.clone(),
            inbound_sender,
        ));
        Self {
            client: AcpClient {
                writer,
                pending,
                next_request_id: Arc::new(AtomicI64::new(1)),
                trace_sessions,
            },
            inbound,
        }
    }

    /// Receives the next session event in transport order.
    pub async fn next_event(&mut self) -> Option<AcpInboundEvent> {
        self.inbound.recv().await
    }

    /// Splits the peer into its writer client and ordered inbound receiver.
    pub fn into_parts(self) -> (AcpClient<Writer>, mpsc::UnboundedReceiver<AcpInboundEvent>) {
        (self.client, self.inbound)
    }
}

/// Parses agent frames and routes responses, updates, and requests without blocking on consumers.
async fn read_frames<Reader, Writer>(
    reader: Reader,
    writer: Arc<AsyncMutex<Writer>>,
    pending: Arc<Mutex<PendingRequests>>,
    trace_sessions: SessionTraceRegistry,
    inbound: mpsc::UnboundedSender<AcpInboundEvent>,
) where
    Reader: AsyncRead + Unpin,
    Writer: AsyncWrite + Unpin,
{
    let mut lines = FramedRead::new(reader, LinesCodec::new_with_max_length(MAX_FRAME_BYTES));
    while let Some(line) = lines.next().await {
        let value = match line {
            Ok(line) => match serde_json::from_str::<Value>(&line) {
                Ok(value) => value,
                Err(error) => {
                    let _ = inbound.send(AcpInboundEvent::Fatal(AcpError::InvalidFrame(
                        error.to_string(),
                    )));
                    lock_pending(&pending).clear();
                    return;
                }
            },
            Err(LinesCodecError::MaxLineLengthExceeded) => {
                let _ = inbound.send(AcpInboundEvent::Fatal(AcpError::FrameTooLarge));
                lock_pending(&pending).clear();
                return;
            }
            Err(LinesCodecError::Io(error)) => {
                let _ = inbound.send(AcpInboundEvent::Fatal(AcpError::Io(error)));
                lock_pending(&pending).clear();
                return;
            }
        };
        #[cfg(debug_assertions)]
        {
            let (msg, jsonrpc_method, session_id) =
                trace_frame_summary(&value, "recv", &trace_sessions, Some(&pending));
            ora_trace!(
                direction = "recv",
                jsonrpc_method = %jsonrpc_method,
                session_id = %session_id,
                frame = %value,
                "{}", msg,
            );
        }
        if let Err(error) = route_frame(value, &writer, &pending, &trace_sessions, &inbound).await {
            let _ = inbound.send(AcpInboundEvent::Fatal(error));
            lock_pending(&pending).clear();
            return;
        }
    }
    let _ = inbound.send(AcpInboundEvent::Fatal(AcpError::StreamClosed));
    // Retaining these senders would turn a known EOF into unrelated outer timeouts.
    lock_pending(&pending).clear();
}

/// Routes one validated JSON-RPC object and makes ambiguous shapes fatal.
async fn route_frame<Writer>(
    value: Value,
    writer: &AsyncMutex<Writer>,
    pending: &Mutex<PendingRequests>,
    trace_sessions: &SessionTraceRegistry,
    inbound: &mpsc::UnboundedSender<AcpInboundEvent>,
) -> Result<(), AcpError>
where
    Writer: AsyncWrite + Unpin,
{
    let object = value.as_object().ok_or_else(|| {
        AcpError::InvalidFrame("batch and non-object frames are unsupported".to_string())
    })?;
    if object.get("jsonrpc") != Some(&Value::String("2.0".to_string())) {
        return Err(AcpError::InvalidFrame("jsonrpc must equal 2.0".to_string()));
    }
    let method = object.get("method").and_then(Value::as_str);
    let id = object
        .get("id")
        .cloned()
        .map(serde_json::from_value::<RequestId>)
        .transpose()
        .map_err(|error| AcpError::InvalidFrame(error.to_string()))?;

    match (method, id) {
        (Some(method), Some(request_id))
            if method == CLIENT_METHOD_NAMES.session_request_permission =>
        {
            let request =
                serde_json::from_value(object.get("params").cloned().unwrap_or(Value::Null))
                    .map_err(|error| AcpError::InvalidFrame(error.to_string()))?;
            inbound
                .send(AcpInboundEvent::PermissionRequest(PermissionRequest {
                    request_id,
                    request,
                }))
                .map_err(|_| AcpError::StreamClosed)
        }
        (Some(method), Some(request_id)) => {
            // ACP can grow new client methods independently. JSON-RPC requires a correlated
            // method-not-found response, while terminating here would make extensions fatal.
            let response = json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {
                    "code": -32601,
                    "message": format!("method not found: {method}"),
                },
            });
            write_frame(writer, trace_sessions, &response).await
        }
        (Some(method), None) if method == CLIENT_METHOD_NAMES.session_update => {
            let notification =
                serde_json::from_value(object.get("params").cloned().unwrap_or(Value::Null))
                    .map_err(|error| AcpError::InvalidFrame(error.to_string()))?;
            inbound
                .send(AcpInboundEvent::SessionUpdate(notification))
                .map_err(|_| AcpError::StreamClosed)
        }
        (Some(_), None) => Ok(()),
        (None, Some(request_id)) => {
            let response = if let Some(result) = object.get("result") {
                Ok(result.clone())
            } else if let Some(error) = object.get("error") {
                Err(serde_json::from_value(error.clone())
                    .map_err(|parse_error| AcpError::InvalidFrame(parse_error.to_string()))?)
            } else {
                return Err(AcpError::InvalidFrame(
                    "response has neither result nor error".to_string(),
                ));
            };
            let request = match lock_pending(pending).take_response(&request_id) {
                ResponseRequest::Pending(request) => request,
                ResponseRequest::Abandoned => return Ok(()),
                ResponseRequest::Unmatched => {
                    return Err(AcpError::InvalidFrame(format!(
                        "unmatched response id {request_id}"
                    )));
                }
            };
            match request {
                PendingRequest::Direct(sender) => {
                    let _ = sender.send(response);
                    Ok(())
                }
                PendingRequest::Session { session_id } => inbound
                    .send(AcpInboundEvent::SessionResponse(SessionResponse {
                        request_id,
                        session_id,
                        response,
                    }))
                    .map_err(|_| AcpError::StreamClosed),
            }
        }
        (None, None) => Err(AcpError::InvalidFrame(
            "frame has neither method nor id".to_string(),
        )),
    }
}

/// Extracts summary fields from a JSON-RPC frame for trace-level correlation without re-parsing.
#[cfg(debug_assertions)]
fn trace_frame_summary(
    value: &Value,
    direction: &str,
    trace_sessions: &SessionTraceRegistry,
    pending: Option<&Mutex<PendingRequests>>,
) -> (String, String, String) {
    let jsonrpc_method = value.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let agent_session_id = value
        .get("params")
        .and_then(|p| p.get("sessionId"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            let request_id = value
                .get("id")
                .cloned()
                .and_then(|id| serde_json::from_value::<RequestId>(id).ok())?;
            pending
                .map(lock_pending)?
                .session_id(&request_id)
                .map(ToString::to_string)
        })
        .unwrap_or_default();
    let session_id = trace_sessions.resolve(&agent_session_id);
    let is_response = value.get("result").is_some();
    let is_error = value.get("error").is_some();

    let message = if !jsonrpc_method.is_empty() {
        format!("{direction} {jsonrpc_method}")
    } else if is_response {
        format!("{direction} response")
    } else if is_error {
        format!("{direction} error response")
    } else {
        format!("{direction} frame")
    };

    (message, jsonrpc_method.to_string(), session_id)
}

/// Writes a reader-originated protocol response through the connection's serialized sink.
async fn write_frame<Writer>(
    writer: &AsyncMutex<Writer>,
    trace_sessions: &SessionTraceRegistry,
    value: &Value,
) -> Result<(), AcpError>
where
    Writer: AsyncWrite + Unpin,
{
    #[cfg(debug_assertions)]
    {
        let (msg, jsonrpc_method, session_id) =
            trace_frame_summary(value, "send", trace_sessions, /*pending*/ None);
        ora_trace!(
            direction = "send",
            jsonrpc_method = %jsonrpc_method,
            session_id = %session_id,
            frame = %value,
            "{}", msg,
        );
    }
    let mut bytes =
        serde_json::to_vec(value).map_err(|error| AcpError::InvalidFrame(error.to_string()))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(AcpError::FrameTooLarge);
    }
    bytes.push(b'\n');
    let mut writer = writer.lock().await;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AcpError, AcpInboundEvent, AcpPeer, PendingRequest, PendingRequests, trace_frame_summary,
    };
    use crate::trace::SessionTraceRegistry;
    use agent_client_protocol_schema::v1::RequestId;
    use agent_client_protocol_schema::v1::SessionId;
    use agent_client_protocol_schema::v1::SessionNotification;
    use agent_client_protocol_schema::v1::{SessionInfoUpdate, SessionUpdate};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex, split};

    /// Verifies ACP trace fields expose the registered Ora session identity.
    #[test]
    fn traces_the_registered_ora_session_id() {
        let trace_sessions = SessionTraceRegistry::default();
        let _registration = trace_sessions.register("agent-session-1", "ora-session-1");
        let frame = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": "agent-session-1" },
        });

        assert_eq!(
            trace_frame_summary(&frame, "recv", &trace_sessions, /*pending*/ None),
            (
                "recv session/update".to_string(),
                "session/update".to_string(),
                "ora-session-1".to_string(),
            )
        );
    }

    /// Verifies response frames inherit identity from their pending session request.
    #[test]
    fn traces_the_ora_session_id_on_correlated_responses() {
        let trace_sessions = SessionTraceRegistry::default();
        let _registration = trace_sessions.register("agent-session-1", "ora-session-1");
        let request_id = RequestId::Number(7);
        let mut pending = PendingRequests::default();
        pending.insert(
            request_id.clone(),
            PendingRequest::Session {
                session_id: SessionId::new("agent-session-1"),
            },
        );
        let pending = Mutex::new(pending);
        let frame = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": { "stopReason": "end_turn" },
        });

        assert_eq!(
            trace_frame_summary(&frame, "recv", &trace_sessions, Some(&pending)),
            (
                "recv response".to_string(),
                String::new(),
                "ora-session-1".to_string(),
            )
        );
    }

    /// Verifies connection-wide handoff cannot make one burst terminate unrelated sessions.
    #[tokio::test]
    async fn hands_off_more_than_one_session_queue_of_updates() {
        let (ora_stream, mut agent_stream) = duplex(16 * 1024);
        let (ora_reader, ora_writer) = split(ora_stream);
        let mut peer = AcpPeer::spawn(ora_reader, ora_writer);
        let expected = (0..300)
            .map(|index| {
                SessionNotification::new(
                    format!("session-{}", index % 2),
                    SessionUpdate::SessionInfoUpdate(
                        SessionInfoUpdate::new().title(format!("Update {index}")),
                    ),
                )
            })
            .collect::<Vec<_>>();

        for notification in &expected {
            let frame = json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": notification,
            });
            agent_stream
                .write_all(format!("{frame}\n").as_bytes())
                .await
                .expect("write session update");
        }

        let mut received = Vec::with_capacity(expected.len());
        for _ in 0..expected.len() {
            match peer.next_event().await.expect("receive session event") {
                AcpInboundEvent::SessionUpdate(update) => received.push(update),
                AcpInboundEvent::PermissionRequest(_)
                | AcpInboundEvent::SessionResponse(_)
                | AcpInboundEvent::Fatal(_) => panic!("expected session update"),
            }
        }
        assert_eq!(received, expected);
    }

    /// Verifies tail updates and their terminating response preserve transport order.
    #[tokio::test]
    async fn orders_session_updates_before_the_session_response() {
        let (ora_stream, agent_stream) = duplex(16 * 1024);
        let (ora_reader, ora_writer) = split(ora_stream);
        let (agent_reader, mut agent_writer) = split(agent_stream);
        let mut agent_reader = BufReader::new(agent_reader);
        let mut peer = AcpPeer::spawn(ora_reader, ora_writer);
        let session_id = SessionId::new("session-1");
        let pending = peer
            .client
            .start_session_request::<_, Value>(
                session_id.clone(),
                "session/prompt",
                &json!({ "sessionId": session_id }),
            )
            .await
            .expect("start session request");
        let mut outbound = String::new();
        agent_reader
            .read_line(&mut outbound)
            .await
            .expect("read session request");
        let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse session request");
        let request_id = outbound["id"].clone();
        let expected = ["First", "Second"].map(|title| {
            SessionNotification::new(
                "session-1",
                SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title(title)),
            )
        });
        for update in &expected {
            agent_writer
                .write_all(
                    format!(
                        "{}\n",
                        json!({
                            "jsonrpc": "2.0",
                            "method": "session/update",
                            "params": update,
                        })
                    )
                    .as_bytes(),
                )
                .await
                .expect("write session update");
        }
        agent_writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "result": { "stopReason": "end_turn" },
                    })
                )
                .as_bytes(),
            )
            .await
            .expect("write session response");

        for expected_update in expected {
            match peer.next_event().await.expect("receive update event") {
                AcpInboundEvent::SessionUpdate(update) => assert_eq!(update, expected_update),
                AcpInboundEvent::PermissionRequest(_)
                | AcpInboundEvent::SessionResponse(_)
                | AcpInboundEvent::Fatal(_) => panic!("expected session update"),
            }
        }
        let response = match peer.next_event().await.expect("receive response event") {
            AcpInboundEvent::SessionResponse(response) => response,
            AcpInboundEvent::SessionUpdate(_)
            | AcpInboundEvent::PermissionRequest(_)
            | AcpInboundEvent::Fatal(_) => panic!("expected session response"),
        };
        assert_eq!(
            pending.finish(response).expect("finish session request"),
            json!({ "stopReason": "end_turn" })
        );
    }

    /// Verifies abandoning a session request discards its late response without a fatal.
    #[tokio::test]
    async fn discards_a_late_response_after_the_session_request_is_abandoned() {
        let (ora_stream, agent_stream) = duplex(16 * 1024);
        let (ora_reader, ora_writer) = split(ora_stream);
        let (agent_reader, mut agent_writer) = split(agent_stream);
        let mut agent_reader = BufReader::new(agent_reader);
        let mut peer = AcpPeer::spawn(ora_reader, ora_writer);
        let session_id = SessionId::new("session-1");
        let pending = peer
            .client
            .start_session_request::<_, Value>(
                session_id.clone(),
                "session/prompt",
                &json!({ "sessionId": session_id }),
            )
            .await
            .expect("start session request");
        let mut outbound = String::new();
        agent_reader
            .read_line(&mut outbound)
            .await
            .expect("read session request");
        let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse session request");
        pending.abandon();
        agent_writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "id": outbound["id"],
                        "result": { "stopReason": "end_turn" },
                    })
                )
                .as_bytes(),
            )
            .await
            .expect("write abandoned session response");
        agent_writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": SessionNotification::new(
                            "session-1",
                            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Alive")),
                        ),
                    })
                )
                .as_bytes(),
            )
            .await
            .expect("write follow-up update");

        match peer.next_event().await.expect("receive follow-up update") {
            AcpInboundEvent::SessionUpdate(update) => assert_eq!(
                update,
                SessionNotification::new(
                    "session-1",
                    SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Alive")),
                )
            ),
            AcpInboundEvent::PermissionRequest(_)
            | AcpInboundEvent::SessionResponse(_)
            | AcpInboundEvent::Fatal(_) => {
                panic!("expected session update after abandoned response")
            }
        }
    }

    /// Verifies dropping an unsettled handle unregisters the request like an explicit abandon.
    #[tokio::test]
    async fn dropping_a_pending_session_request_unregisters_it() {
        let (ora_stream, agent_stream) = duplex(16 * 1024);
        let (ora_reader, ora_writer) = split(ora_stream);
        let (agent_reader, mut agent_writer) = split(agent_stream);
        let mut agent_reader = BufReader::new(agent_reader);
        let mut peer = AcpPeer::spawn(ora_reader, ora_writer);
        let session_id = SessionId::new("session-1");
        let pending = peer
            .client
            .start_session_request::<_, Value>(
                session_id.clone(),
                "session/prompt",
                &json!({ "sessionId": session_id }),
            )
            .await
            .expect("start session request");
        let mut outbound = String::new();
        agent_reader
            .read_line(&mut outbound)
            .await
            .expect("read session request");
        let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse session request");
        drop(pending);
        agent_writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "id": outbound["id"],
                        "result": { "stopReason": "cancelled" },
                    })
                )
                .as_bytes(),
            )
            .await
            .expect("write dropped session response");
        agent_writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": SessionNotification::new(
                            "session-1",
                            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Still open")),
                        ),
                    })
                )
                .as_bytes(),
            )
            .await
            .expect("write follow-up update");

        match peer.next_event().await.expect("receive follow-up update") {
            AcpInboundEvent::SessionUpdate(update) => assert_eq!(
                update,
                SessionNotification::new(
                    "session-1",
                    SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Still open")),
                )
            ),
            AcpInboundEvent::PermissionRequest(_)
            | AcpInboundEvent::SessionResponse(_)
            | AcpInboundEvent::Fatal(_) => panic!("expected session update after dropped request"),
        }
    }

    /// Verifies cancelling a direct request retires its id and keeps later traffic readable.
    #[tokio::test]
    async fn dropping_a_direct_request_future_unregisters_it() {
        let (ora_stream, agent_stream) = duplex(16 * 1024);
        let (ora_reader, ora_writer) = split(ora_stream);
        let (agent_reader, mut agent_writer) = split(agent_stream);
        let mut agent_reader = BufReader::new(agent_reader);
        let mut peer = AcpPeer::spawn(ora_reader, ora_writer);
        let client = peer.client.clone();
        let request = tokio::spawn(async move {
            client
                .request::<_, Value>("session/list", &json!({ "cwd": "/workspace" }))
                .await
        });

        let mut outbound = String::new();
        agent_reader
            .read_line(&mut outbound)
            .await
            .expect("read direct request");
        let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse direct request");
        request.abort();
        let _ = request.await;

        agent_writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "id": outbound["id"],
                        "result": { "sessions": [] },
                    })
                )
                .as_bytes(),
            )
            .await
            .expect("write abandoned direct response");
        agent_writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": SessionNotification::new(
                            "session-1",
                            SessionUpdate::SessionInfoUpdate(
                                SessionInfoUpdate::new().title("Still connected"),
                            ),
                        ),
                    })
                )
                .as_bytes(),
            )
            .await
            .expect("write follow-up update");

        match peer.next_event().await.expect("receive follow-up update") {
            AcpInboundEvent::SessionUpdate(update) => assert_eq!(
                update,
                SessionNotification::new(
                    "session-1",
                    SessionUpdate::SessionInfoUpdate(
                        SessionInfoUpdate::new().title("Still connected"),
                    ),
                )
            ),
            AcpInboundEvent::PermissionRequest(_)
            | AcpInboundEvent::SessionResponse(_)
            | AcpInboundEvent::Fatal(_) => {
                panic!("expected update after abandoned direct response")
            }
        }
    }

    /// Verifies a response id that was never pending remains a fatal correlation failure.
    #[tokio::test]
    async fn rejects_a_response_with_an_unknown_id() {
        let (ora_stream, agent_stream) = duplex(16 * 1024);
        let (ora_reader, ora_writer) = split(ora_stream);
        let (agent_reader, mut agent_writer) = split(agent_stream);
        let mut agent_reader = BufReader::new(agent_reader);
        let mut peer = AcpPeer::spawn(ora_reader, ora_writer);
        let session_id = SessionId::new("session-1");
        let _pending = peer
            .client
            .start_session_request::<_, Value>(
                session_id.clone(),
                "session/prompt",
                &json!({ "sessionId": session_id }),
            )
            .await
            .expect("start session request");
        let mut outbound = String::new();
        agent_reader
            .read_line(&mut outbound)
            .await
            .expect("read session request");
        agent_writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "id": 999,
                        "result": { "stopReason": "end_turn" },
                    })
                )
                .as_bytes(),
            )
            .await
            .expect("write unmatched response");

        match peer.next_event().await.expect("receive fatal event") {
            AcpInboundEvent::Fatal(AcpError::InvalidFrame(message)) => {
                assert_eq!(message, "unmatched response id 999");
            }
            AcpInboundEvent::SessionUpdate(_)
            | AcpInboundEvent::PermissionRequest(_)
            | AcpInboundEvent::SessionResponse(_)
            | AcpInboundEvent::Fatal(_) => panic!("expected unmatched response failure"),
        }
    }

    /// Verifies extension requests receive method-not-found without closing request correlation.
    #[tokio::test]
    async fn rejects_unknown_agent_request_and_continues_reading() {
        let (ora_stream, agent_stream) = duplex(16 * 1024);
        let (ora_reader, ora_writer) = split(ora_stream);
        let (agent_reader, mut agent_writer) = split(agent_stream);
        let mut agent_reader = BufReader::new(agent_reader);
        let peer = AcpPeer::spawn(ora_reader, ora_writer);
        let client = peer.client.clone();
        let request = tokio::spawn(async move {
            client
                .request::<_, Value>("initialize", &json!({ "protocolVersion": 1 }))
                .await
        });

        let mut outbound = String::new();
        agent_reader
            .read_line(&mut outbound)
            .await
            .expect("read Ora request");
        let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse Ora request");
        let request_id = outbound["id"].clone();
        agent_writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"ext/future\",\"params\":{}}\n")
            .await
            .expect("write extension request");

        let mut rejection = String::new();
        agent_reader
            .read_line(&mut rejection)
            .await
            .expect("read method-not-found response");
        assert_eq!(
            serde_json::from_str::<Value>(rejection.trim()).expect("parse rejection"),
            json!({
                "jsonrpc": "2.0",
                "id": 99,
                "error": {
                    "code": -32601,
                    "message": "method not found: ext/future",
                },
            })
        );

        let response = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": { "accepted": true },
        });
        agent_writer
            .write_all(format!("{response}\n").as_bytes())
            .await
            .expect("write correlated response");
        assert_eq!(
            request
                .await
                .expect("join request")
                .expect("complete request"),
            json!({ "accepted": true })
        );
    }

    /// Verifies EOF wakes correlated requests instead of leaving them to an outer timeout.
    #[tokio::test]
    async fn closes_pending_requests_when_agent_stdout_ends() {
        let (ora_stream, agent_stream) = duplex(16 * 1024);
        let (ora_reader, ora_writer) = split(ora_stream);
        let (agent_reader, mut agent_writer) = split(agent_stream);
        let mut agent_reader = BufReader::new(agent_reader);
        let peer = AcpPeer::spawn(ora_reader, ora_writer);
        let client = peer.client.clone();
        let request = tokio::spawn(async move {
            client
                .request::<_, Value>("initialize", &json!({ "protocolVersion": 1 }))
                .await
        });
        let mut outbound = String::new();
        agent_reader
            .read_line(&mut outbound)
            .await
            .expect("read Ora request");

        agent_writer.shutdown().await.expect("close agent writer");
        drop(agent_reader);
        drop(agent_writer);

        assert!(matches!(
            request.await.expect("join request"),
            Err(AcpError::StreamClosed)
        ));
    }
}
