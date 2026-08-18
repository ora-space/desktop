use std::sync::atomic::AtomicI64;
use std::sync::{Arc, Mutex};

use agent_client_protocol_schema::v1::CLIENT_METHOD_NAMES;
use agent_client_protocol_schema::v1::RequestId;
use ora_logging::ora_trace;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::client::AcpClient;
use crate::error::AcpError;
use crate::events::{AcpInboundEvent, PermissionRequest, SessionResponse};
use crate::frame::send_traced;
use crate::pending::{PendingRequest, PendingRequests, ResponseRequest, lock_pending};
use crate::trace::SessionTraceRegistry;
use crate::transport::{AcpMessages, AcpTransport};

#[cfg(debug_assertions)]
use crate::frame::trace_frame_summary;

/// Owns the ordered inbound receiver for one ACP connection.
pub struct AcpPeer<Transport> {
    pub client: AcpClient<Transport>,
    inbound: mpsc::UnboundedReceiver<AcpInboundEvent>,
}

impl<Transport> AcpPeer<Transport>
where
    Transport: AcpTransport,
{
    /// Starts the routing task and delegates session-event flow control to the connection owner.
    pub fn spawn(messages: AcpMessages, transport: Transport) -> Self {
        let pending = Arc::new(Mutex::new(PendingRequests::default()));
        let transport = Arc::new(transport);
        let trace_sessions = SessionTraceRegistry::default();
        // The application router applies bounded queues per provider session. Bounding this
        // connection-wide handoff would let one noisy session terminate every other session.
        let (inbound_sender, inbound) = mpsc::unbounded_channel();
        tokio::spawn(route_messages(
            messages,
            transport.clone(),
            pending.clone(),
            trace_sessions.clone(),
            inbound_sender,
        ));
        Self {
            client: AcpClient {
                transport,
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
    pub fn into_parts(
        self,
    ) -> (
        AcpClient<Transport>,
        mpsc::UnboundedReceiver<AcpInboundEvent>,
    ) {
        (self.client, self.inbound)
    }
}

/// Routes decoded messages into responses, updates, and requests without blocking on consumers.
async fn route_messages<Transport>(
    mut messages: AcpMessages,
    transport: Arc<Transport>,
    pending: Arc<Mutex<PendingRequests>>,
    trace_sessions: SessionTraceRegistry,
    inbound: mpsc::UnboundedSender<AcpInboundEvent>,
) where
    Transport: AcpTransport,
{
    while let Some(message) = messages.recv().await {
        let value = match message {
            Ok(value) => value,
            Err(error) => {
                let _ = inbound.send(AcpInboundEvent::Fatal(error));
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
        if let Err(error) =
            route_frame(value, &transport, &pending, &trace_sessions, &inbound).await
        {
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
async fn route_frame<Transport>(
    value: Value,
    transport: &Arc<Transport>,
    pending: &Mutex<PendingRequests>,
    trace_sessions: &SessionTraceRegistry,
    inbound: &mpsc::UnboundedSender<AcpInboundEvent>,
) -> Result<(), AcpError>
where
    Transport: AcpTransport,
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
            send_traced(transport, trace_sessions, response).await
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
