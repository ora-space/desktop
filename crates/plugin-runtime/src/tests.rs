use std::collections::HashSet;
use std::io;
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use ora_process::ManagedProcess;
use pretty_assertions::assert_eq;
use serde_json::json;
use tokio::io::duplex;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};
use tokio::time::timeout;

use crate::host_requests::{HostRequestError, HostRequestHandler, NoHostRequests};
use crate::protocol::{PluginNotification, PluginRegistration, handle_message as handle};
use crate::state::{PendingRequests, RuntimeInner, RuntimeStatus};
use crate::tasks::{run_supervisor, run_writer};
use crate::{PluginProcessExit, PluginRuntime, PluginRuntimeError, RuntimeLease};

/// Applies one message with no host-request handler, the shape most protocol tests need.
async fn handle_message(inner: &RuntimeInner, message: serde_json::Value) -> Result<(), String> {
    handle(inner, &Arc::new(NoHostRequests), message).await
}

/// Builds one isolated protocol state whose inbound notifications the caller can observe.
fn test_inner() -> (RuntimeInner, mpsc::UnboundedReceiver<PluginNotification>) {
    let (inner, inbound_rx, _writer_rx) = test_inner_with_writer();
    (inner, inbound_rx)
}

/// Builds one isolated protocol state that also exposes the frames queued for the writer task.
fn test_inner_with_writer() -> (
    RuntimeInner,
    mpsc::UnboundedReceiver<PluginNotification>,
    mpsc::Receiver<serde_json::Value>,
) {
    let (status_tx, _) = watch::channel(RuntimeStatus::Starting);
    let (exited_tx, _) = watch::channel(false);
    let (writer_tx, writer_rx) = mpsc::channel(8);
    let (supervisor_tx, _) = mpsc::unbounded_channel();
    let (inbound, inbound_rx) = mpsc::unbounded_channel();
    let inner = RuntimeInner {
        plugin_id: "example".to_string(),
        registration: RwLock::new(PluginRegistration::default()),
        status_tx,
        exited_tx,
        writer_tx,
        supervisor_tx,
        inbound: Mutex::new(Some(inbound)),
        pending: Mutex::new(PendingRequests::default()),
        next_request_id: AtomicU64::new(1),
        call_timeout: Duration::from_secs(5),
    };
    (inner, inbound_rx, writer_rx)
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

/// Echoes params back, but holds the response of any request whose params ask for a gate until
/// that gate opens, so tests can force answers to complete out of arrival order.
struct GatedEcho {
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

impl HostRequestHandler for GatedEcho {
    async fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, HostRequestError> {
        match method {
            "host/echo" => {
                if params.get("wait").is_some()
                    && let Some(gate) = self.gate.lock().await.take()
                {
                    let _ = gate.await;
                }
                Ok(params)
            }
            "host/fail" => {
                Err(HostRequestError::new(-32000, "storage failed")
                    .with_data(json!({ "kind": "io" })))
            }
            other => Err(HostRequestError::method_not_found(other)),
        }
    }
}

/// A request before registration is a protocol violation, not something to queue or answer.
#[tokio::test]
async fn rejects_plugin_requests_before_registration() {
    let (inner, _inbound) = test_inner();

    let error = handle_message(
        &inner,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "host/echo" }),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error,
        "plugin sent request host/echo before completing registration"
    );
}

/// A request id that is neither a number nor a string cannot be echoed back safely.
#[tokio::test]
async fn rejects_plugin_requests_with_invalid_ids() {
    let (inner, _inbound) = test_inner();
    register(&inner, "example.echo", "example.tick").await;

    let error = handle_message(
        &inner,
        json!({ "jsonrpc": "2.0", "id": { "nested": 1 }, "method": "host/echo" }),
    )
    .await
    .unwrap_err();

    assert_eq!(error, "plugin request host/echo has an invalid request ID");
}

/// Requests run concurrently and each response carries the id of the request it answers, even
/// when a later request finishes first; unknown methods and handler failures become JSON-RPC
/// errors under the same id.
#[tokio::test]
async fn answers_plugin_requests_by_id_independently_of_completion_order() {
    let (inner, _inbound, mut writer_rx) = test_inner_with_writer();
    register(&inner, "example.echo", "example.tick").await;
    let (open_gate, gate) = oneshot::channel();
    let handler = Arc::new(GatedEcho {
        gate: Mutex::new(Some(gate)),
    });

    for message in [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "host/echo", "params": { "wait": true } }),
        json!({ "jsonrpc": "2.0", "id": "two", "method": "host/echo", "params": { "n": 2 } }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "host/missing" }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "host/fail" }),
    ] {
        handle(&inner, &handler, message).await.unwrap();
    }
    let mut responses = Vec::new();
    for _ in 0..3 {
        responses.push(writer_rx.recv().await.unwrap());
    }
    open_gate.send(()).unwrap();
    responses.push(writer_rx.recv().await.unwrap());
    // The three ungated answers race each other; only the gated one has a fixed position.
    responses[..3].sort_by_key(|response| response["id"].to_string());

    assert_eq!(
        responses,
        vec![
            json!({ "jsonrpc": "2.0", "id": "two", "result": { "n": 2 } }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "error": { "code": -32601, "message": "unknown host method host/missing" },
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "error": { "code": -32000, "message": "storage failed", "data": { "kind": "io" } },
            }),
            json!({ "jsonrpc": "2.0", "id": 1, "result": { "wait": true } }),
        ]
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
    assert!(matches!(
        inner.pending.lock().await.take_response(9),
        crate::state::ResponseRequest::Unmatched
    ));
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

/// A late response to a timed-out call is ignored without invalidating the plugin generation.
#[tokio::test]
async fn ignores_a_late_response_to_an_abandoned_request() {
    let (status_tx, _) = watch::channel(RuntimeStatus::Ready);
    let (exited_tx, _) = watch::channel(false);
    let (writer_tx, _writer_rx) = mpsc::channel(1);
    let (supervisor_tx, _supervisor_rx) = mpsc::unbounded_channel();
    let (inbound, _inbound_rx) = mpsc::unbounded_channel();
    let inner = Arc::new(RuntimeInner {
        plugin_id: "example".to_string(),
        registration: RwLock::new(PluginRegistration {
            methods: HashSet::from(["example.echo".to_string()]),
            emits: HashSet::new(),
        }),
        status_tx,
        exited_tx,
        writer_tx: writer_tx.clone(),
        supervisor_tx: supervisor_tx.clone(),
        inbound: Mutex::new(Some(inbound)),
        pending: Mutex::new(PendingRequests::default()),
        next_request_id: AtomicU64::new(11),
        call_timeout: Duration::from_millis(1),
    });
    let runtime = PluginRuntime {
        inner: Arc::clone(&inner),
        _lease: Arc::new(RuntimeLease {
            writer_tx,
            supervisor_tx,
        }),
    };

    assert_eq!(
        runtime.invoke("example.echo", json!({})).await,
        Err(PluginRuntimeError::CallTimeout)
    );

    handle_message(
        &inner,
        json!({ "jsonrpc": "2.0", "id": 11, "result": "late" }),
    )
    .await
    .expect("ignore late response");

    assert_eq!(*inner.status_tx.borrow(), RuntimeStatus::Ready);
}

/// A response id the host never issued remains a protocol violation.
#[tokio::test]
async fn rejects_a_genuinely_unknown_response_id() {
    let (inner, _inbound) = test_inner();

    assert_eq!(
        handle_message(
            &inner,
            json!({ "jsonrpc": "2.0", "id": 99, "result": "foreign" }),
        )
        .await,
        Err("plugin responded with unknown request ID 99".to_string())
    );
}

struct ControllableProcess {
    exited: watch::Sender<bool>,
    killed: Arc<AtomicBool>,
}

impl ManagedProcess for ControllableProcess {
    type Stdin = tokio::io::DuplexStream;
    type Stdout = tokio::io::DuplexStream;
    type Stderr = tokio::io::DuplexStream;

    fn id(&self) -> Option<u32> {
        None
    }

    fn take_stdin(&mut self) -> Option<Self::Stdin> {
        None
    }

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        None
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        None
    }

    fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        Ok((*self.exited.borrow()).then(successful_exit_status))
    }

    async fn wait(&self) -> io::Result<ExitStatus> {
        let mut exited = self.exited.subscribe();
        while !*exited.borrow() {
            exited
                .changed()
                .await
                .map_err(|_| io::Error::other("process exit channel closed"))?;
        }
        Ok(successful_exit_status())
    }

    async fn kill(&self) -> io::Result<()> {
        self.killed.store(true, Ordering::Release);
        self.exited.send_replace(true);
        Ok(())
    }
}

/// Builds a successful platform exit status for the controllable process.
fn successful_exit_status() -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }
}

/// Shutdown-and-wait does not return until the real supervisor confirms full process-tree exit.
#[tokio::test]
async fn shutdown_and_wait_blocks_until_the_process_is_reaped() {
    let (status_tx, _) = watch::channel(RuntimeStatus::Ready);
    let (exited_tx, _) = watch::channel(false);
    let (writer_tx, _writer_rx) = mpsc::channel(1);
    let (supervisor_tx, supervisor_rx) = mpsc::unbounded_channel();
    let (inbound, mut inbound_rx) = mpsc::unbounded_channel();
    let inner = Arc::new(RuntimeInner {
        plugin_id: "example".to_string(),
        registration: RwLock::new(PluginRegistration::default()),
        status_tx,
        exited_tx,
        writer_tx: writer_tx.clone(),
        supervisor_tx: supervisor_tx.clone(),
        inbound: Mutex::new(Some(inbound)),
        pending: Mutex::new(PendingRequests::default()),
        next_request_id: AtomicU64::new(1),
        call_timeout: Duration::from_secs(5),
    });
    let runtime = PluginRuntime {
        inner: Arc::clone(&inner),
        _lease: Arc::new(RuntimeLease {
            writer_tx,
            supervisor_tx,
        }),
    };
    let (process_exited, _) = watch::channel(false);
    let killed = Arc::new(AtomicBool::new(false));
    let process = ControllableProcess {
        exited: process_exited.clone(),
        killed: Arc::clone(&killed),
    };
    let (writer_close, _writer_close_rx) = oneshot::channel();
    let supervisor = tokio::spawn(run_supervisor(
        process,
        supervisor_rx,
        Arc::clone(&inner),
        Duration::from_secs(1),
        writer_close,
    ));

    let shutdown = tokio::spawn(async move { runtime.shutdown_and_wait().await });
    tokio::task::yield_now().await;
    assert!(!shutdown.is_finished());

    process_exited.send_replace(true);
    assert_eq!(shutdown.await.unwrap(), PluginProcessExit::Stopped);
    supervisor.await.unwrap();
    assert!(!killed.load(Ordering::Acquire));
    assert_eq!(inbound_rx.recv().await, None);
}

/// Unexpected process exit fails the generation and closes its upper-layer message stream.
#[tokio::test]
async fn process_exit_is_visible_to_the_connection_owner() {
    let (status_tx, _) = watch::channel(RuntimeStatus::Ready);
    let (exited_tx, _) = watch::channel(false);
    let (writer_tx, _writer_rx) = mpsc::channel(1);
    let (supervisor_tx, supervisor_rx) = mpsc::unbounded_channel();
    let (inbound, mut inbound_rx) = mpsc::unbounded_channel();
    let inner = Arc::new(RuntimeInner {
        plugin_id: "example".to_string(),
        registration: RwLock::new(PluginRegistration::default()),
        status_tx,
        exited_tx,
        writer_tx,
        supervisor_tx,
        inbound: Mutex::new(Some(inbound)),
        pending: Mutex::new(PendingRequests::default()),
        next_request_id: AtomicU64::new(1),
        call_timeout: Duration::from_secs(5),
    });
    let (process_exited, _) = watch::channel(false);
    let killed = Arc::new(AtomicBool::new(false));
    let process = ControllableProcess {
        exited: process_exited.clone(),
        killed: Arc::clone(&killed),
    };
    let (writer_close, _writer_close_rx) = oneshot::channel();
    let supervisor = tokio::spawn(run_supervisor(
        process,
        supervisor_rx,
        Arc::clone(&inner),
        Duration::from_secs(1),
        writer_close,
    ));

    process_exited.send_replace(true);
    supervisor.await.unwrap();

    assert_eq!(
        *inner.status_tx.borrow(),
        RuntimeStatus::Failed(format!(
            "plugin process exited with {}",
            successful_exit_status()
        ))
    );
    assert!(!killed.load(Ordering::Acquire));
    assert_eq!(inbound_rx.recv().await, None);
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

/// Captures the spec of every spawn while handing out a process without stdio pipes.
///
/// Missing pipes make `launch` fail right after spawning, which is exactly enough to audit the
/// argv, working directory, and environment the runtime derives from its configuration.
struct CapturingSpawner {
    specs: std::sync::Mutex<Vec<ora_process::ProcessSpec>>,
}

impl ora_process::ProcessSpawner for CapturingSpawner {
    type Process = ControllableProcess;

    fn spawn(&self, spec: ora_process::ProcessSpec) -> io::Result<Self::Process> {
        self.specs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(spec);
        let (exited, _) = watch::channel(false);
        Ok(ControllableProcess {
            exited,
            killed: Arc::new(AtomicBool::new(false)),
        })
    }
}

/// Permissions and working directory both reach the spawned process spec.
#[tokio::test]
async fn launch_applies_permissions_and_cwd_to_the_process_spec() {
    let package_root = tempfile::tempdir().expect("create package root");
    let entrypoint = package_root.path().join("index.js");
    std::fs::write(&entrypoint, "export {};\n").expect("write entrypoint");
    let spawner = CapturingSpawner {
        specs: std::sync::Mutex::new(Vec::new()),
    };

    let error = PluginRuntime::launch(
        &spawner,
        crate::PluginRuntimeConfig {
            plugin_id: "example".to_string(),
            deno_path: "deno".into(),
            entrypoint: entrypoint.clone(),
            permissions: vec!["--allow-read=/tmp/data".to_string()],
            cwd: Some(package_root.path().to_path_buf()),
            ready_timeout: Duration::from_secs(1),
            call_timeout: Duration::from_secs(1),
            shutdown_timeout: Duration::from_secs(1),
        },
        NoHostRequests,
    )
    .await
    .map(|_| ())
    .unwrap_err();
    assert_eq!(error, PluginRuntimeError::MissingStdio);

    let specs = spawner
        .specs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let expected = ora_process::ProcessSpec::new("deno")
        .arg("run")
        .arg("--no-prompt")
        .arg("--allow-read=/tmp/data")
        .arg(entrypoint.as_os_str())
        .cwd(package_root.path());
    assert_eq!(*specs, vec![expected]);
}
