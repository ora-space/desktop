use std::sync::Arc;
use std::time::Duration;

use ora_logging::{ora_error, ora_warn};
use ora_plugin_protocol::{read_message, write_message};
use ora_process::ManagedProcess;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use crate::PluginRuntimeError;
use crate::host_requests::HostRequestHandler;
use crate::plugin_log::{self, PluginLogPipeline};
use crate::protocol::handle_message;
use crate::state::{
    RuntimeInner, RuntimeStatus, SupervisorCommand, close_inbound, fail_pending, fail_runtime,
};

/// Serializes all outbound frames through one task so concurrent callers cannot interleave bytes.
pub(crate) async fn run_writer<W>(
    mut stdin: W,
    mut messages: mpsc::Receiver<Value>,
    mut close: oneshot::Receiver<()>,
    inner: Arc<RuntimeInner>,
) where
    W: AsyncWrite + Unpin,
{
    loop {
        let message = tokio::select! {
            message = messages.recv() => match message {
                Some(message) => message,
                None => return,
            },
            _ = &mut close => return,
        };
        if let Err(error) = write_message(&mut stdin, &message).await {
            fail_runtime(&inner, format!("failed to write plugin frame: {error}")).await;
            return;
        }
    }
}

/// Reads plugin registration, notifications, requests, and responses from the stdout stream.
pub(crate) async fn run_reader<R, H>(mut stdout: R, inner: Arc<RuntimeInner>, host_requests: Arc<H>)
where
    R: AsyncRead + Unpin,
    H: HostRequestHandler,
{
    loop {
        let message = match read_message(&mut stdout).await {
            Ok(Some(message)) => message,
            Ok(None) => {
                fail_runtime(&inner, "plugin stdout closed".to_string()).await;
                return;
            }
            Err(error) => {
                fail_runtime(&inner, format!("invalid plugin frame: {error}")).await;
                return;
            }
        };
        if let Err(reason) = handle_message(&inner, &host_requests, message).await {
            fail_runtime(&inner, reason).await;
            return;
        }
    }
}

/// Supervises process exit and guarantees a bounded graceful shutdown.
///
/// The generation is only reported as exited once its plugin log has also finished: a stop
/// that returned while the log writer still held the active file would let the next generation
/// or an uninstall race it.
pub(crate) async fn run_supervisor<P>(
    process: P,
    mut commands: mpsc::UnboundedReceiver<SupervisorCommand>,
    inner: Arc<RuntimeInner>,
    shutdown_timeout: Duration,
    writer_close: oneshot::Sender<()>,
    plugin_log: PluginLogPipeline,
    log_teardown_timeout: Duration,
) where
    P: ManagedProcess + Send + 'static,
{
    tokio::select! {
        status = process.wait() => {
            let reason = match status {
                Ok(status) => format!("plugin process exited with {status}"),
                Err(error) => format!("failed to wait for plugin process: {error}"),
            };
            if !matches!(*inner.status_tx.borrow(), RuntimeStatus::ShuttingDown) {
                ora_warn!(
                    plugin_id = %inner.plugin_id,
                    reason = %reason,
                    "plugin process exited unexpectedly"
                );
            }
            fail_pending(&inner, PluginRuntimeError::Unavailable(reason.clone())).await;
            if !matches!(*inner.status_tx.borrow(), RuntimeStatus::ShuttingDown) {
                inner.status_tx.send_replace(RuntimeStatus::Failed(reason));
            }
        }
        command = commands.recv() => {
            let graceful_exit = match command {
                Some(SupervisorCommand::Shutdown) => {
                    inner.status_tx.send_replace(RuntimeStatus::ShuttingDown);
                    timeout(shutdown_timeout, process.wait()).await.is_ok()
                }
                Some(SupervisorCommand::ProtocolFailure) => false,
                None => {
                    inner.status_tx.send_replace(RuntimeStatus::ShuttingDown);
                    false
                }
            };
            if !graceful_exit {
                if let Err(error) = process.kill().await {
                    ora_error!(
                        message = "failed to terminate plugin process tree",
                        plugin_id = %inner.plugin_id,
                        error = %error,
                    );
                }
                let _ = process.wait().await;
            }
            fail_pending(
                &inner,
                PluginRuntimeError::Unavailable("plugin stopped".to_string()),
            ).await;
        }
    }
    close_inbound(&inner).await;
    let _ = writer_close.send(());
    plugin_log::finish(plugin_log, log_teardown_timeout).await;
    inner.exited_tx.send_replace(true);
}
