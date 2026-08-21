//! Plugin-to-host requests: the reverse of `invoke`, served by a handler the launcher injects.
//!
//! A plugin may send JSON-RPC requests (messages with an `id`) once it is registered. The
//! runtime hands each one to the launch-time [`HostRequestHandler`] on its own task, so a slow
//! handler never stalls the reader that also carries responses to the host's own calls, and
//! writes the handler's answer back under the plugin's id. The runtime itself knows no host
//! method: which methods exist, and what they do, is entirely the handler's decision.

use std::future::Future;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use crate::protocol::JSON_RPC_VERSION;

/// JSON-RPC code for a method the host does not serve.
pub const METHOD_NOT_FOUND_CODE: i64 = -32601;

/// One failed host request, rendered as a JSON-RPC error object on the wire.
///
/// `data` is the structured part of the error: handlers put machine-readable classification
/// there (for example a `kind` string) so plugins can branch without parsing `message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRequestError {
    code: i64,
    message: String,
    data: Value,
}

impl HostRequestError {
    /// Creates an error with a human-readable message and no structured data.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: Value::Null,
        }
    }

    /// Attaches structured data that is serialized as the JSON-RPC `data` member.
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    /// The error every handler returns for a method it does not serve.
    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            METHOD_NOT_FOUND_CODE,
            format!("unknown host method {method}"),
        )
    }

    /// The JSON-RPC error code.
    pub fn code(&self) -> i64 {
        self.code
    }

    /// The human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The structured `data` member; `Null` when the error carries none.
    pub fn data(&self) -> &Value {
        &self.data
    }

    /// Renders the JSON-RPC error object; `data` is omitted when there is none.
    fn to_json(&self) -> Value {
        let mut object = Map::new();
        object.insert("code".to_string(), json!(self.code));
        object.insert("message".to_string(), json!(self.message));
        if !self.data.is_null() {
            object.insert("data".to_string(), self.data.clone());
        }
        Value::Object(object)
    }
}

/// Serves the requests one plugin process sends to the host.
///
/// One handler instance is bound to one launched process, which is how a handler knows the
/// caller's identity without trusting request params: the launcher constructs it for that plugin
/// alone. Implementations must return `HostRequestError::method_not_found` for methods they do
/// not serve and must be safe to call concurrently, because every request runs on its own task.
pub trait HostRequestHandler: Send + Sync + 'static {
    /// Answers one request; the result becomes the JSON-RPC `result` member.
    fn handle(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, HostRequestError>> + Send;
}

/// Handler for launches whose plugin contract has no host-side methods at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoHostRequests;

impl HostRequestHandler for NoHostRequests {
    /// Rejects every method so a plugin learns immediately that nothing is served here.
    async fn handle(&self, method: &str, _params: Value) -> Result<Value, HostRequestError> {
        Err(HostRequestError::method_not_found(method))
    }
}

/// Runs one plugin request to completion and queues its response for the writer task.
///
/// A closed writer means the process generation is already ending, so the unsent response is
/// dropped silently: the plugin that asked is gone too.
pub(crate) async fn serve_request<H: HostRequestHandler>(
    handler: Arc<H>,
    writer_tx: mpsc::Sender<Value>,
    request_id: Value,
    method: String,
    params: Value,
) {
    let response = match handler.handle(&method, params).await {
        Ok(result) => json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": request_id,
            "result": result,
        }),
        Err(error) => json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": request_id,
            "error": error.to_json(),
        }),
    };
    let _ = writer_tx.send(response).await;
}
