mod codec;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ora_logging::{ora_error, ora_info, ora_warn};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};
use tokio::time::timeout;

use crate::codec::{read_frame, write_frame};

const JSON_RPC_VERSION: &str = "2.0";
const REGISTER_METHOD: &str = "ora/register";
const SHUTDOWN_METHOD: &str = "ora/shutdown";

/// Describes one eagerly started Deno plugin process and its lifecycle timeouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRuntimeConfig {
    pub plugin_id: String,
    pub deno_path: PathBuf,
    pub entrypoint: PathBuf,
    pub ready_timeout: Duration,
    pub call_timeout: Duration,
    pub shutdown_timeout: Duration,
}

/// Reports why a plugin cannot start or serve a method invocation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PluginRuntimeError {
    #[error("plugin entrypoint does not exist: {0}")]
    MissingEntrypoint(PathBuf),
    #[error("failed to start plugin process: {0}")]
    Spawn(String),
    #[error("plugin process did not expose all required stdio pipes")]
    MissingStdio,
    #[error("plugin did not register methods before the startup deadline")]
    ReadyTimeout,
    #[error("plugin is unavailable: {0}")]
    Unavailable(String),
    #[error("plugin did not register method {0}")]
    MethodNotRegistered(String),
    #[error("plugin request channel is closed")]
    RequestChannelClosed,
    #[error("plugin method call timed out")]
    CallTimeout,
    #[error("plugin method failed with code {code}: {message}")]
    Remote { code: i64, message: String },
}

/// Reports whether the supervised plugin exited intentionally or failed unexpectedly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginProcessExit {
    Stopped,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeStatus {
    Starting,
    Ready,
    Failed(String),
    ShuttingDown,
}

type PendingResult = Result<Value, PluginRuntimeError>;

struct RuntimeInner {
    plugin_id: String,
    methods: RwLock<HashSet<String>>,
    status_tx: watch::Sender<RuntimeStatus>,
    exited_tx: watch::Sender<bool>,
    writer_tx: mpsc::Sender<Value>,
    supervisor_tx: mpsc::UnboundedSender<SupervisorCommand>,
    pending: Mutex<HashMap<u64, oneshot::Sender<PendingResult>>>,
    next_request_id: AtomicU64,
    call_timeout: Duration,
}

struct RuntimeLease {
    writer_tx: mpsc::Sender<Value>,
    supervisor_tx: mpsc::UnboundedSender<SupervisorCommand>,
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        let _ = self.writer_tx.try_send(json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": SHUTDOWN_METHOD,
        }));
        let _ = self.supervisor_tx.send(SupervisorCommand::Shutdown);
    }
}

/// Owns a ready plugin connection and correlates concurrent method calls by request ID.
#[derive(Clone)]
pub struct PluginRuntime {
    inner: Arc<RuntimeInner>,
    _lease: Arc<RuntimeLease>,
}

impl PluginRuntime {
    /// Launches one plugin and waits until it publishes its immutable method registration.
    pub async fn launch<P>(
        spawner: &P,
        config: PluginRuntimeConfig,
    ) -> Result<Self, PluginRuntimeError>
    where
        P: ProcessSpawner,
        P::Process: Send + 'static,
    {
        if !config.entrypoint.is_file() {
            return Err(PluginRuntimeError::MissingEntrypoint(config.entrypoint));
        }

        let spec = ProcessSpec::new(config.deno_path.as_os_str())
            .arg("run")
            .arg("--no-prompt")
            .arg(config.entrypoint.as_os_str());
        let mut process = spawner
            .spawn(spec)
            .map_err(|error| PluginRuntimeError::Spawn(error.to_string()))?;
        let Some(stdin) = process.take_stdin() else {
            return Err(PluginRuntimeError::MissingStdio);
        };
        let Some(stdout) = process.take_stdout() else {
            return Err(PluginRuntimeError::MissingStdio);
        };
        let Some(stderr) = process.take_stderr() else {
            return Err(PluginRuntimeError::MissingStdio);
        };

        let (writer_tx, writer_rx) = mpsc::channel(64);
        let (writer_close_tx, writer_close_rx) = oneshot::channel();
        let (supervisor_tx, supervisor_rx) = mpsc::unbounded_channel();
        let (status_tx, mut status_rx) = watch::channel(RuntimeStatus::Starting);
        let (exited_tx, _) = watch::channel(false);
        let inner = Arc::new(RuntimeInner {
            plugin_id: config.plugin_id.clone(),
            methods: RwLock::new(HashSet::new()),
            status_tx,
            exited_tx,
            writer_tx: writer_tx.clone(),
            supervisor_tx: supervisor_tx.clone(),
            pending: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            call_timeout: config.call_timeout,
        });
        let runtime = Self {
            inner: Arc::clone(&inner),
            _lease: Arc::new(RuntimeLease {
                writer_tx,
                supervisor_tx,
            }),
        };

        tokio::spawn(run_writer(
            stdin,
            writer_rx,
            writer_close_rx,
            Arc::clone(&inner),
        ));
        tokio::spawn(run_reader(stdout, Arc::clone(&inner)));
        tokio::spawn(run_stderr(stderr, config.plugin_id.clone()));
        tokio::spawn(run_supervisor(
            process,
            supervisor_rx,
            Arc::clone(&inner),
            config.shutdown_timeout,
            writer_close_tx,
        ));

        let ready_result = timeout(config.ready_timeout, async {
            loop {
                match status_rx.borrow().clone() {
                    RuntimeStatus::Starting => {}
                    RuntimeStatus::Ready => return Ok(()),
                    RuntimeStatus::Failed(reason) => {
                        return Err(PluginRuntimeError::Unavailable(reason));
                    }
                    RuntimeStatus::ShuttingDown => {
                        return Err(PluginRuntimeError::Unavailable(
                            "plugin stopped during startup".to_string(),
                        ));
                    }
                }
                status_rx.changed().await.map_err(|_| {
                    PluginRuntimeError::Unavailable("plugin status channel closed".to_string())
                })?;
            }
        })
        .await;

        match ready_result {
            Ok(result) => result?,
            Err(_) => {
                runtime.request_shutdown();
                return Err(PluginRuntimeError::ReadyTimeout);
            }
        }

        ora_info!(
            message = "plugin runtime ready",
            plugin_id = %config.plugin_id,
        );
        Ok(runtime)
    }

    /// Invokes one registered method and returns its JSON result.
    pub async fn invoke(&self, method: &str, params: Value) -> Result<Value, PluginRuntimeError> {
        match self.inner.status_tx.borrow().clone() {
            RuntimeStatus::Ready => {}
            RuntimeStatus::Starting => {
                return Err(PluginRuntimeError::Unavailable(
                    "plugin is still starting".to_string(),
                ));
            }
            RuntimeStatus::Failed(reason) => {
                return Err(PluginRuntimeError::Unavailable(reason));
            }
            RuntimeStatus::ShuttingDown => {
                return Err(PluginRuntimeError::Unavailable(
                    "plugin is shutting down".to_string(),
                ));
            }
        }
        if !self.inner.methods.read().await.contains(method) {
            return Err(PluginRuntimeError::MethodNotRegistered(method.to_string()));
        }

        let request_id = self.inner.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (result_tx, result_rx) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .await
            .insert(request_id, result_tx);
        let request = json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": request_id,
            "method": method,
            "params": params,
        });
        if self.inner.writer_tx.send(request).await.is_err() {
            self.inner.pending.lock().await.remove(&request_id);
            return Err(PluginRuntimeError::RequestChannelClosed);
        }

        match timeout(self.inner.call_timeout, result_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(PluginRuntimeError::Unavailable(
                "plugin stopped before responding".to_string(),
            )),
            Err(_) => {
                self.inner.pending.lock().await.remove(&request_id);
                Err(PluginRuntimeError::CallTimeout)
            }
        }
    }

    /// Requests graceful shutdown and resolves only after the supervised process has exited.
    pub async fn shutdown(&self) {
        self.request_shutdown();
        let mut exited = self.inner.exited_tx.subscribe();
        while !*exited.borrow() && exited.changed().await.is_ok() {}
    }

    /// Waits for process exit and classifies intentional shutdown separately from failure.
    pub async fn wait_for_exit(&self) -> PluginProcessExit {
        let mut exited = self.inner.exited_tx.subscribe();
        while !*exited.borrow() && exited.changed().await.is_ok() {}
        match self.inner.status_tx.borrow().clone() {
            RuntimeStatus::Failed(reason) => PluginProcessExit::Failed(reason),
            RuntimeStatus::Starting | RuntimeStatus::Ready | RuntimeStatus::ShuttingDown => {
                PluginProcessExit::Stopped
            }
        }
    }

    fn request_shutdown(&self) {
        let _ = self.inner.writer_tx.try_send(json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": SHUTDOWN_METHOD,
        }));
        let _ = self.inner.supervisor_tx.send(SupervisorCommand::Shutdown);
    }
}

#[derive(Debug, Clone, Copy)]
enum SupervisorCommand {
    Shutdown,
    ProtocolFailure,
}

/// Serializes all outbound frames through one task so concurrent callers cannot interleave bytes.
async fn run_writer<W>(
    mut stdin: W,
    mut messages: mpsc::Receiver<Value>,
    mut close: oneshot::Receiver<()>,
    inner: Arc<RuntimeInner>,
) where
    W: tokio::io::AsyncWrite + Unpin,
{
    loop {
        let message = tokio::select! {
            message = messages.recv() => match message {
                Some(message) => message,
                None => return,
            },
            _ = &mut close => return,
        };
        let payload = match serde_json::to_vec(&message) {
            Ok(payload) => payload,
            Err(error) => {
                fail_runtime(&inner, format!("failed to encode plugin request: {error}")).await;
                return;
            }
        };
        if let Err(error) = write_frame(&mut stdin, &payload).await {
            fail_runtime(&inner, format!("failed to write plugin frame: {error}")).await;
            return;
        }
    }
}

/// Reads plugin registration and responses from the stdout protocol stream.
async fn run_reader<R>(mut stdout: R, inner: Arc<RuntimeInner>)
where
    R: AsyncRead + Unpin,
{
    loop {
        let payload = match read_frame(&mut stdout).await {
            Ok(Some(payload)) => payload,
            Ok(None) => {
                fail_runtime(&inner, "plugin stdout closed".to_string()).await;
                return;
            }
            Err(error) => {
                fail_runtime(&inner, format!("invalid plugin frame: {error}")).await;
                return;
            }
        };
        let message: Value = match serde_json::from_slice(&payload) {
            Ok(message) => message,
            Err(error) => {
                fail_runtime(&inner, format!("invalid plugin JSON: {error}")).await;
                return;
            }
        };
        if let Err(reason) = handle_message(&inner, message).await {
            fail_runtime(&inner, reason).await;
            return;
        }
    }
}

/// Applies one validated registration or response message to runtime state.
async fn handle_message(inner: &RuntimeInner, message: Value) -> Result<(), String> {
    let object = message
        .as_object()
        .ok_or_else(|| "plugin message must be a JSON object".to_string())?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some(JSON_RPC_VERSION) {
        return Err("plugin message has an invalid JSON-RPC version".to_string());
    }

    if object.get("method").and_then(Value::as_str) == Some(REGISTER_METHOD) {
        if object.contains_key("id") {
            return Err("plugin registration must be a notification".to_string());
        }
        if !matches!(*inner.status_tx.borrow(), RuntimeStatus::Starting) {
            return Err("plugin registered methods more than once".to_string());
        }
        let methods = object
            .get("params")
            .and_then(|params| params.get("methods"))
            .and_then(Value::as_array)
            .ok_or_else(|| "plugin registration is missing a methods array".to_string())?;
        let mut registered = HashSet::with_capacity(methods.len());
        for method in methods {
            let method = method
                .as_str()
                .filter(|method| !method.is_empty())
                .ok_or_else(|| "plugin registration contains an invalid method".to_string())?;
            if !registered.insert(method.to_string()) {
                return Err(format!("plugin registered duplicate method {method}"));
            }
        }
        *inner.methods.write().await = registered;
        inner.status_tx.send_replace(RuntimeStatus::Ready);
        return Ok(());
    }

    if object.contains_key("method") {
        return Err("plugin sent an unsupported notification or request".to_string());
    }

    let request_id = object
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "plugin response has an invalid request ID".to_string())?;
    let result = match (object.get("result"), object.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) => {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .ok_or_else(|| "plugin error response has an invalid code".to_string())?;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| "plugin error response has an invalid message".to_string())?;
            Err(PluginRuntimeError::Remote {
                code,
                message: message.to_string(),
            })
        }
        _ => return Err("plugin response must contain exactly one result or error".to_string()),
    };
    let sender = inner
        .pending
        .lock()
        .await
        .remove(&request_id)
        .ok_or_else(|| format!("plugin responded with unknown request ID {request_id}"))?;
    let _ = sender.send(result);
    Ok(())
}

/// Drains plugin stderr continuously so logging cannot block the child process.
async fn run_stderr<R>(mut stderr: R, plugin_id: String)
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) => return,
            Ok(length) => {
                let message = String::from_utf8_lossy(&buffer[..length]);
                ora_info!(
                    message = "plugin stderr",
                    plugin_id = %plugin_id,
                    output = %message.trim_end(),
                );
            }
            Err(error) => {
                ora_warn!(
                    message = "failed to read plugin stderr",
                    plugin_id = %plugin_id,
                    error = %error,
                );
                return;
            }
        }
    }
}

/// Supervises process exit and guarantees a bounded graceful shutdown.
async fn run_supervisor<P>(
    process: P,
    mut commands: mpsc::UnboundedReceiver<SupervisorCommand>,
    inner: Arc<RuntimeInner>,
    shutdown_timeout: Duration,
    writer_close: oneshot::Sender<()>,
) where
    P: ManagedProcess + Send + 'static,
{
    tokio::select! {
        status = process.wait() => {
            let reason = match status {
                Ok(status) => format!("plugin process exited with {status}"),
                Err(error) => format!("failed to wait for plugin process: {error}"),
            };
            fail_pending(&inner, PluginRuntimeError::Unavailable(reason.clone())).await;
            if !matches!(*inner.status_tx.borrow(), RuntimeStatus::ShuttingDown) {
                inner.status_tx.send_replace(RuntimeStatus::Failed(reason));
            }
        }
        command = commands.recv() => {
            // Protocol failures already carry the actionable reason; overwriting them with
            // ShuttingDown would make lifecycle observers misclassify a crash as an explicit stop.
            if matches!(command, Some(SupervisorCommand::Shutdown)) {
                inner.status_tx.send_replace(RuntimeStatus::ShuttingDown);
            }
            let stopped_gracefully = matches!(command, Some(SupervisorCommand::Shutdown))
                && timeout(shutdown_timeout, process.wait()).await.is_ok();
            if !stopped_gracefully {
                if let Err(error) = process.kill().await {
                    ora_error!(
                        message = "failed to terminate plugin process tree",
                        plugin_id = %inner.plugin_id,
                        error = %error,
                    );
                }
                let _ = process.wait().await;
            }
        }
    }
    let _ = writer_close.send(());
    inner.exited_tx.send_replace(true);
}

/// Marks a protocol connection unusable and wakes every waiting caller.
async fn fail_runtime(inner: &RuntimeInner, reason: String) {
    inner
        .status_tx
        .send_replace(RuntimeStatus::Failed(reason.clone()));
    fail_pending(inner, PluginRuntimeError::Unavailable(reason)).await;
    let _ = inner.supervisor_tx.send(SupervisorCommand::ProtocolFailure);
}

/// Completes all pending requests with the same terminal runtime failure.
async fn fail_pending(inner: &RuntimeInner, error: PluginRuntimeError) {
    let pending = std::mem::take(&mut *inner.pending.lock().await);
    for sender in pending.into_values() {
        let _ = sender.send(Err(error.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeInner, RuntimeStatus, handle_message, run_writer};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::AtomicU64;
    use std::time::Duration;
    use tokio::io::duplex;
    use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};
    use tokio::time::timeout;

    fn test_inner() -> RuntimeInner {
        let (status_tx, _) = watch::channel(RuntimeStatus::Starting);
        let (exited_tx, _) = watch::channel(false);
        let (writer_tx, _) = mpsc::channel(1);
        let (supervisor_tx, _) = mpsc::unbounded_channel();
        RuntimeInner {
            plugin_id: "example".to_string(),
            methods: RwLock::new(HashSet::new()),
            status_tx,
            exited_tx,
            writer_tx,
            supervisor_tx,
            pending: Mutex::new(HashMap::new()),
            next_request_id: AtomicU64::new(1),
            call_timeout: Duration::from_secs(5),
        }
    }

    /// Registration atomically publishes the complete immutable method set.
    #[tokio::test]
    async fn accepts_initial_registration() {
        let inner = test_inner();

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
            inner.methods.read().await.clone(),
            HashSet::from(["example.echo".to_string()])
        );
        assert_eq!(*inner.status_tx.borrow(), RuntimeStatus::Ready);
    }

    /// Duplicate method names invalidate registration rather than selecting one handler.
    #[tokio::test]
    async fn rejects_duplicate_registration() {
        let inner = test_inner();

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

        assert_eq!(error, "plugin registered duplicate method example.echo");
    }

    /// A response resolves only the pending caller with the matching numeric ID.
    #[tokio::test]
    async fn routes_response_by_request_id() {
        let inner = test_inner();
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

    /// Lets the supervisor end an idle writer task after the child process exits.
    #[tokio::test]
    async fn closes_idle_writer_on_supervisor_signal() {
        let inner = std::sync::Arc::new(test_inner());
        let (stdin, _host_reader) = duplex(64);
        let (_messages, message_rx) = mpsc::channel(1);
        let (close_tx, close_rx) = oneshot::channel();
        let writer = tokio::spawn(run_writer(stdin, message_rx, close_rx, inner));

        close_tx.send(()).unwrap();

        timeout(Duration::from_secs(1), writer)
            .await
            .unwrap()
            .unwrap();
    }
}
