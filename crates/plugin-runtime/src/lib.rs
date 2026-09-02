//! Owns the lifecycle and bidirectional stdio protocol of one sandboxed Ora plugin process.

mod host_requests;
mod protocol;
mod state;
mod tasks;

#[cfg(test)]
mod tests;

pub use host_requests::{
    HostRequestError, HostRequestHandler, METHOD_NOT_FOUND_CODE, NoHostRequests,
};
pub use ora_plugin_protocol::{PluginEffectCoordination, PluginEffectResource, PluginRegistration};
pub use protocol::PluginNotification;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ora_logging::ora_info;
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot, watch};
use tokio::time::timeout;

use crate::state::{PendingRequests, RuntimeInner, RuntimeStatus, SupervisorCommand};
use ora_plugin_protocol::{JSON_RPC_VERSION, SHUTDOWN_METHOD};

/// Describes one eagerly started Deno plugin process and its lifecycle timeouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRuntimeConfig {
    pub plugin_id: String,
    pub deno_path: PathBuf,
    pub entrypoint: PathBuf,
    /// Extra Deno permission flags placed before the entrypoint, such as `--allow-run`.
    pub permissions: Vec<String>,
    /// Working directory of the plugin process, normally its package root so that relative
    /// imports and configuration discovery resolve against the package instead of the host.
    pub cwd: Option<PathBuf>,
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

/// Ends the plugin process once the last clone of its public handle is dropped.
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
    /// Launches one plugin and waits until it publishes its immutable capability registration.
    ///
    /// `host_requests` serves every request the plugin sends to the host for the life of this
    /// process; it is bound here, at launch, so the handler knows which plugin is calling
    /// without ever reading an identity from request params.
    ///
    /// The returned receiver carries every whitelisted plugin-originated notification. It is
    /// unbounded on purpose: connection-wide backpressure would let one noisy stream stall
    /// unrelated traffic on the same process, so bounded queues belong to each consumer instead.
    pub async fn launch<P, H>(
        spawner: &P,
        config: PluginRuntimeConfig,
        host_requests: H,
    ) -> Result<(Self, mpsc::UnboundedReceiver<PluginNotification>), PluginRuntimeError>
    where
        P: ProcessSpawner,
        P::Process: Send + 'static,
        H: HostRequestHandler,
    {
        if !config.entrypoint.is_file() {
            return Err(PluginRuntimeError::MissingEntrypoint(config.entrypoint));
        }

        let mut spec = ProcessSpec::new(config.deno_path.as_os_str())
            .arg("run")
            .arg("--no-prompt")
            .args(config.permissions.iter().map(String::as_str))
            .arg(config.entrypoint.as_os_str());
        if let Some(cwd) = &config.cwd {
            spec = spec.cwd(cwd);
        }
        let mut process = spawner
            .spawn(spec)
            .map_err(|error| PluginRuntimeError::Spawn(error.to_string()))?;
        let stdio = (
            process.take_stdin(),
            process.take_stdout(),
            process.take_stderr(),
        );
        let (Some(stdin), Some(stdout), Some(stderr)) = stdio else {
            let _ = process.kill().await;
            let _ = process.wait().await;
            return Err(PluginRuntimeError::MissingStdio);
        };

        let (writer_tx, writer_rx) = mpsc::channel(64);
        let (writer_close_tx, writer_close_rx) = oneshot::channel();
        let (supervisor_tx, supervisor_rx) = mpsc::unbounded_channel();
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        let (status_tx, mut status_rx) = watch::channel(RuntimeStatus::Starting);
        let (exited_tx, _) = watch::channel(false);
        let inner = Arc::new(RuntimeInner {
            plugin_id: config.plugin_id.clone(),
            registration: RwLock::new(PluginRegistration::default()),
            status_tx,
            exited_tx,
            writer_tx: writer_tx.clone(),
            supervisor_tx: supervisor_tx.clone(),
            inbound: Mutex::new(Some(inbound_tx)),
            pending: Mutex::new(PendingRequests::default()),
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

        tokio::spawn(tasks::run_writer(
            stdin,
            writer_rx,
            writer_close_rx,
            Arc::clone(&inner),
        ));
        tokio::spawn(tasks::run_reader(
            stdout,
            Arc::clone(&inner),
            Arc::new(host_requests),
        ));
        tokio::spawn(tasks::run_stderr(stderr, config.plugin_id.clone()));
        tokio::spawn(tasks::run_supervisor(
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
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                runtime.shutdown_and_wait().await;
                return Err(error);
            }
            Err(_) => {
                runtime.shutdown_and_wait().await;
                return Err(PluginRuntimeError::ReadyTimeout);
            }
        }

        ora_info!(
            message = "plugin runtime ready",
            plugin_id = %config.plugin_id,
        );
        Ok((runtime, inbound_rx))
    }

    /// Returns the capability declaration published by the plugin, fixed once the plugin is ready.
    pub async fn registration(&self) -> PluginRegistration {
        self.inner.registration.read().await.clone()
    }

    /// Invokes one registered method and returns its JSON result.
    pub async fn invoke(&self, method: &str, params: Value) -> Result<Value, PluginRuntimeError> {
        self.invoke_with_timeout(method, params, self.inner.call_timeout)
            .await
    }

    /// Invokes one registered method with a budget chosen for that operation.
    ///
    /// Heavy control operations such as model discovery may need to start a child process, so
    /// forcing them through the short-call budget would either make discovery unreliable or
    /// weaken the timeout applied to every ordinary control call.
    pub async fn invoke_with_timeout(
        &self,
        method: &str,
        params: Value,
        call_timeout: Duration,
    ) -> Result<Value, PluginRuntimeError> {
        self.ensure_ready()?;
        if !self
            .inner
            .registration
            .read()
            .await
            .methods
            .contains(method)
        {
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
            self.inner.pending.lock().await.remove_active(request_id);
            return Err(PluginRuntimeError::RequestChannelClosed);
        }

        match timeout(call_timeout, result_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(PluginRuntimeError::Unavailable(
                "plugin stopped before responding".to_string(),
            )),
            Err(_) => {
                self.inner.pending.lock().await.abandon(request_id);
                Err(PluginRuntimeError::CallTimeout)
            }
        }
    }

    /// Sends one host-originated notification the plugin never answers.
    ///
    /// Notifications occupy no correlation slot and are therefore outside `call_timeout`, which
    /// exists to bound short control calls. Payloads that stream for minutes, such as ACP
    /// traffic, depend on that distinction.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), PluginRuntimeError> {
        self.ensure_ready()?;
        let notification = json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": method,
            "params": params,
        });
        self.inner
            .writer_tx
            .send(notification)
            .await
            .map_err(|_| PluginRuntimeError::RequestChannelClosed)
    }

    /// Rejects traffic whenever the connection is not serving, carrying why it stopped serving.
    fn ensure_ready(&self) -> Result<(), PluginRuntimeError> {
        match self.inner.status_tx.borrow().clone() {
            RuntimeStatus::Ready => Ok(()),
            RuntimeStatus::Starting => Err(PluginRuntimeError::Unavailable(
                "plugin is still starting".to_string(),
            )),
            RuntimeStatus::Failed(reason) => Err(PluginRuntimeError::Unavailable(reason)),
            RuntimeStatus::ShuttingDown => Err(PluginRuntimeError::Unavailable(
                "plugin is shutting down".to_string(),
            )),
        }
    }

    /// Ends the plugin process without waiting for the last handle clone to be dropped.
    ///
    /// Handles are cloned into long-lived callers, so drop alone cannot bound teardown: one
    /// surviving clone would keep a failed plugin running. Callers that own a plugin's lifetime
    /// therefore end it explicitly, and the supervisor still kills the process tree if the plugin
    /// does not exit within its shutdown timeout.
    pub fn shutdown(&self) {
        self.request_shutdown();
    }

    /// Requests bounded process-tree shutdown and returns only after the child has been reaped.
    ///
    /// Lifecycle owners use this instead of `shutdown` when starting a replacement generation:
    /// the blocking boundary prevents old and new plugin generations from overlapping.
    pub async fn shutdown_and_wait(&self) -> PluginProcessExit {
        self.request_shutdown();
        self.wait_for_exit().await
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
