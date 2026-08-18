use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use pretty_assertions::assert_eq;
use serde_json::json;
use tokio::io::duplex;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};
use tokio::time::timeout;

use crate::protocol::{PluginNotification, PluginRegistration, handle_message};
use crate::state::{RuntimeInner, RuntimeStatus};
use crate::tasks::run_writer;

/// Builds one isolated protocol state whose inbound notifications the caller can observe.
fn test_inner() -> (RuntimeInner, mpsc::UnboundedReceiver<PluginNotification>) {
    let (status_tx, _) = watch::channel(RuntimeStatus::Starting);
    let (exited_tx, _) = watch::channel(false);
    let (writer_tx, _) = mpsc::channel(1);
    let (supervisor_tx, _) = mpsc::unbounded_channel();
    let (inbound, inbound_rx) = mpsc::unbounded_channel();
    let inner = RuntimeInner {
        plugin_id: "example".to_string(),
        registration: RwLock::new(PluginRegistration::default()),
        status_tx,
        exited_tx,
        writer_tx,
        supervisor_tx,
        inbound,
        pending: Mutex::new(HashMap::new()),
        next_request_id: AtomicU64::new(1),
        call_timeout: Duration::from_secs(5),
    };
    (inner, inbound_rx)
}

/// Registers a plugin that may both serve `method` and emit `emit`.
async fn register(inner: &RuntimeInner, method: &str, emit: &str) {
    handle_message(
        inner,
        json!({
            "jsonrpc": "2.0",
            "method": "ora/register",
            "params": { "methods": [method], "emits": [emit] },
        }),
    )
    .await
    .expect("register plugin");
}

/// Registration atomically publishes both directions of the immutable capability declaration.
#[tokio::test]
async fn accepts_initial_registration() {
    let (inner, _inbound) = test_inner();

    handle_message(
        &inner,
        json!({
            "jsonrpc": "2.0",
            "method": "ora/register",
            "params": { "methods": ["example.echo"], "emits": ["example.tick"] },
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        inner.registration.read().await.clone(),
        PluginRegistration {
            methods: HashSet::from(["example.echo".to_string()]),
            emits: HashSet::from(["example.tick".to_string()]),
        }
    );
    assert_eq!(*inner.status_tx.borrow(), RuntimeStatus::Ready);
}

/// A plugin that never emits stays valid, so `emits` is optional rather than required.
#[tokio::test]
async fn defaults_missing_emits_to_an_empty_whitelist() {
    let (inner, _inbound) = test_inner();

    handle_message(
        &inner,
        json!({
            "jsonrpc": "2.0",
            "method": "ora/register",
            "params": { "methods": ["example.echo"] },
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        inner.registration.read().await.clone(),
        PluginRegistration {
            methods: HashSet::from(["example.echo".to_string()]),
            emits: HashSet::new(),
        }
    );
}

/// Duplicate method names invalidate registration rather than selecting one handler.
#[tokio::test]
async fn rejects_duplicate_registration() {
    let (inner, _inbound) = test_inner();

    let error = handle_message(
        &inner,
        json!({
            "jsonrpc": "2.0",
            "method": "ora/register",
            "params": { "methods": ["example.echo", "example.echo"] },
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error,
        "plugin registered duplicate methods entry example.echo"
    );
}

/// A whitelisted notification reaches the host stream with its payload untouched.
#[tokio::test]
async fn delivers_declared_notifications_to_the_inbound_stream() {
    let (inner, mut inbound) = test_inner();
    register(&inner, "example.echo", "example.tick").await;

    handle_message(
        &inner,
        json!({
            "jsonrpc": "2.0",
            "method": "example.tick",
            "params": { "nested": [1, 2, 3] },
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        inbound.recv().await.unwrap(),
        PluginNotification {
            method: "example.tick".to_string(),
            params: json!({ "nested": [1, 2, 3] }),
        }
    );
}

/// A notification outside the declared whitelist invalidates the connection.
#[tokio::test]
async fn rejects_undeclared_notifications() {
    let (inner, _inbound) = test_inner();
    register(&inner, "example.echo", "example.tick").await;

    let error = handle_message(
        &inner,
        json!({ "jsonrpc": "2.0", "method": "example.other" }),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error,
        "plugin sent notification example.other without declaring it in emits"
    );
}

/// Plugins may not open reverse request/response traffic even for a whitelisted method.
#[tokio::test]
async fn rejects_plugin_originated_requests() {
    let (inner, _inbound) = test_inner();
    register(&inner, "example.echo", "example.tick").await;

    let error = handle_message(
        &inner,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "example.tick" }),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error,
        "plugin sent request example.tick; plugins may only send notifications"
    );
}

/// A response resolves only the pending caller with the matching numeric ID.
#[tokio::test]
async fn routes_response_by_request_id() {
    let (inner, _inbound) = test_inner();
    let (sender, receiver) = oneshot::channel();
    inner.pending.lock().await.insert(7, sender);

    handle_message(
        &inner,
        json!({ "jsonrpc": "2.0", "id": 7, "result": "cba" }),
    )
    .await
    .unwrap();

    assert_eq!(receiver.await.unwrap().unwrap(), json!("cba"));
}

/// Notifications interleaved with a response leave correlation state untouched.
#[tokio::test]
async fn keeps_correlation_intact_when_notifications_interleave() {
    let (inner, mut inbound) = test_inner();
    register(&inner, "example.echo", "example.tick").await;
    let (sender, receiver) = oneshot::channel();
    inner.pending.lock().await.insert(9, sender);

    for message in [
        json!({ "jsonrpc": "2.0", "method": "example.tick", "params": 1 }),
        json!({ "jsonrpc": "2.0", "id": 9, "result": "done" }),
        json!({ "jsonrpc": "2.0", "method": "example.tick", "params": 2 }),
    ] {
        handle_message(&inner, message).await.unwrap();
    }

    assert_eq!(receiver.await.unwrap().unwrap(), json!("done"));
    assert!(inner.pending.lock().await.is_empty());
    assert_eq!(
        (inbound.recv().await.unwrap(), inbound.recv().await.unwrap()),
        (
            PluginNotification {
                method: "example.tick".to_string(),
                params: json!(1),
            },
            PluginNotification {
                method: "example.tick".to_string(),
                params: json!(2),
            }
        )
    );
}

/// Lets the supervisor end an idle writer task after the child process exits.
#[tokio::test]
async fn closes_idle_writer_on_supervisor_signal() {
    let (inner, _inbound) = test_inner();
    let (stdin, _host_reader) = duplex(64);
    let (_messages, message_rx) = mpsc::channel(1);
    let (close_tx, close_rx) = oneshot::channel();
    let writer = tokio::spawn(run_writer(stdin, message_rx, close_rx, Arc::new(inner)));

    close_tx.send(()).unwrap();

    timeout(Duration::from_secs(1), writer)
        .await
        .unwrap()
        .unwrap();
}
