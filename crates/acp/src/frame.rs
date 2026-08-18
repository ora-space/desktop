use std::sync::Arc;

use ora_logging::ora_trace;
use serde_json::Value;

use crate::error::AcpError;
use crate::transport::AcpTransport;

#[cfg(debug_assertions)]
use {
    crate::pending::{PendingRequests, lock_pending},
    agent_client_protocol_schema::v1::RequestId,
    std::sync::Mutex,
};

use crate::trace::SessionTraceRegistry;

/// Sends one whole message through the connection transport after recording its trace summary.
pub(crate) async fn send_traced<Transport>(
    transport: &Arc<Transport>,
    trace_sessions: &SessionTraceRegistry,
    value: Value,
) -> Result<(), AcpError>
where
    Transport: AcpTransport,
{
    #[cfg(debug_assertions)]
    {
        let (msg, jsonrpc_method, session_id) =
            trace_frame_summary(&value, "send", trace_sessions, /*pending*/ None);
        ora_trace!(
            direction = "send",
            jsonrpc_method = %jsonrpc_method,
            session_id = %session_id,
            frame = %value,
            "{}", msg,
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = trace_sessions;
    transport.send(value).await
}

/// Extracts summary fields from a JSON-RPC frame for trace-level correlation without re-parsing.
#[cfg(debug_assertions)]
pub(crate) fn trace_frame_summary(
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

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::trace_frame_summary;
    use crate::pending::{PendingRequest, PendingRequests};
    use crate::trace::SessionTraceRegistry;
    use agent_client_protocol_schema::v1::RequestId;
    use agent_client_protocol_schema::v1::SessionId;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Mutex;

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
}
