use std::marker::PhantomData;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol_schema::v1::RequestId;
use agent_client_protocol_schema::v1::SessionId;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::sync::oneshot;

use crate::error::AcpError;
use crate::events::SessionResponse;
use crate::frame::send_traced;
use crate::pending::{PendingRequest, PendingRequests, lock_pending};
use crate::trace::{SessionTraceRegistration, SessionTraceRegistry};
use crate::transport::AcpTransport;

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

/// Sends correlated ACP requests and protocol responses over one connection transport.
pub struct AcpClient<Transport> {
    pub(crate) transport: Arc<Transport>,
    pub(crate) pending: Arc<Mutex<PendingRequests>>,
    pub(crate) next_request_id: Arc<AtomicI64>,
    pub(crate) trace_sessions: SessionTraceRegistry,
}

impl<Transport> Clone for AcpClient<Transport> {
    fn clone(&self) -> Self {
        Self {
            transport: self.transport.clone(),
            pending: self.pending.clone(),
            next_request_id: self.next_request_id.clone(),
            trace_sessions: self.trace_sessions.clone(),
        }
    }
}

impl<Transport> AcpClient<Transport>
where
    Transport: AcpTransport,
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
        if let Err(error) = self.send(frame).await {
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
        if let Err(error) = self.send(frame).await {
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
        self.send(json!({
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
        self.send(json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": result,
        }))
        .await
    }

    /// Hands one complete message to the transport, which owns framing and write ordering.
    async fn send(&self, value: serde_json::Value) -> Result<(), AcpError> {
        send_traced(&self.transport, &self.trace_sessions, value).await
    }
}
