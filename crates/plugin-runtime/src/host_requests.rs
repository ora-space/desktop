//! Plugin-to-host requests: the reverse of `invoke`, served by a handler the launcher injects.
//!
//! A plugin may send JSON-RPC requests (messages with an `id`) once it is registered. The
//! runtime hands each one to the launch-time [`HostRequestHandler`] on its own task, so a slow
//! handler never stalls the reader that also carries responses to the host's own calls, and
//! writes the handler's answer back under the plugin's id. The runtime itself knows no host
//! method: which methods exist, and what they do, is entirely the handler's decision.

use std::future::Future;
use std::pin::Pin;
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

/// One boxed, `'static` host request future.
///
/// The `'static` bound is what keeps [`HostRequestHandler`] object-safe: a handler built once
/// per process can be stored as `dyn` and shared across request tasks without borrowing anything.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Serves the requests one plugin process sends to the host.
///
/// One handler instance is bound to one launched process, which is how a handler knows the
/// caller's identity without trusting request params: the launcher constructs it for that plugin
/// alone. Implementations must return `HostRequestError::method_not_found` for methods they do
/// not serve and must be safe to call concurrently, because every request runs on its own task.
/// The returned future must be `'static`: implementations own (or `Arc`-clone) whatever they
/// need instead of borrowing from `&self`.
pub trait HostRequestHandler: Send + Sync + 'static {
    /// Answers one request; the result becomes the JSON-RPC `result` member.
    fn handle(
        &self,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value, HostRequestError>>;
}

/// Handler for launches whose plugin contract has no host-side methods at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoHostRequests;

impl HostRequestHandler for NoHostRequests {
    /// Rejects every method so a plugin learns immediately that nothing is served here.
    fn handle(
        &self,
        method: &str,
        _params: Value,
    ) -> BoxFuture<'static, Result<Value, HostRequestError>> {
        let method = method.to_owned();
        Box::pin(async move { Err(HostRequestError::method_not_found(&method)) })
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

/// Tries a list of handlers in declaration order, delegating to the next whenever one answers
/// [`METHOD_NOT_FOUND_CODE`]. Any other error — a method the handler knows but refuses — stops
/// the chain, because the refusing handler is authoritative for that method.
pub struct CompositeHostRequests {
    handlers: Vec<Arc<dyn HostRequestHandler>>,
}

impl CompositeHostRequests {
    /// Builds a composite that serves the union of the given handlers' methods.
    pub fn new(handlers: Vec<Arc<dyn HostRequestHandler>>) -> Self {
        Self { handlers }
    }

    /// Whether the composite serves no method at all.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl HostRequestHandler for CompositeHostRequests {
    /// Delegates one request through the handler chain; an exhausted chain is `method_not_found`.
    fn handle(
        &self,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value, HostRequestError>> {
        let handlers = self.handlers.clone();
        let method = method.to_owned();
        Box::pin(async move {
            for handler in &handlers {
                match handler.handle(&method, params.clone()).await {
                    Ok(result) => return Ok(result),
                    Err(error) if error.code() == METHOD_NOT_FOUND_CODE => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(HostRequestError::method_not_found(&method))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HostRequestHandler;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A handler serving exactly one method, counting its invocations.
    struct OneMethod {
        method: &'static str,
        calls: AtomicUsize,
        result: &'static str,
    }

    impl HostRequestHandler for OneMethod {
        fn handle(
            &self,
            method: &str,
            _params: Value,
        ) -> BoxFuture<'static, Result<Value, HostRequestError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let owned_method = method.to_owned();
            let method = self.method;
            let result = self.result;
            Box::pin(async move {
                if owned_method == method {
                    Ok(json!(result))
                } else {
                    Err(HostRequestError::method_not_found(&owned_method))
                }
            })
        }
    }

    /// The composite delegates until a handler owns the method; a refusal short-circuits.
    #[tokio::test]
    async fn composite_delegates_until_a_handler_owns_the_method() {
        let first = Arc::new(OneMethod {
            method: "first/ping",
            calls: AtomicUsize::new(0),
            result: "first",
        });
        let second = Arc::new(OneMethod {
            method: "second/ping",
            calls: AtomicUsize::new(0),
            result: "second",
        });
        let composite = CompositeHostRequests::new(vec![first.clone(), second.clone()]);

        let result = composite
            .handle("second/ping", Value::Null)
            .await
            .expect("composite serves second/ping");
        assert_eq!(result, json!("second"));
        assert_eq!(first.calls.load(Ordering::SeqCst), 1);
        assert_eq!(second.calls.load(Ordering::SeqCst), 1);

        let error = composite
            .handle("nobody/ping", Value::Null)
            .await
            .expect_err("unknown method fails");
        assert_eq!(error.code(), METHOD_NOT_FOUND_CODE);
    }

    /// A known method that refuses with a non-404 error stops the chain.
    #[tokio::test]
    async fn composite_stops_at_the_first_authoritative_refusal() {
        struct Refuser;
        impl HostRequestHandler for Refuser {
            fn handle(
                &self,
                method: &str,
                _params: Value,
            ) -> BoxFuture<'static, Result<Value, HostRequestError>> {
                let method = method.to_owned();
                Box::pin(async move {
                    if method == "refuser/deny" {
                        Err(HostRequestError::new(-32099, "denied"))
                    } else {
                        Err(HostRequestError::method_not_found(&method))
                    }
                })
            }
        }
        let second = Arc::new(OneMethod {
            method: "refuser/deny",
            calls: AtomicUsize::new(0),
            result: "unreachable",
        });
        let composite = CompositeHostRequests::new(vec![Arc::new(Refuser), second.clone()]);

        let error = composite
            .handle("refuser/deny", Value::Null)
            .await
            .expect_err("refusal is authoritative");
        assert_eq!(error.code(), -32099);
        assert_eq!(second.calls.load(Ordering::SeqCst), 0);
    }
}
