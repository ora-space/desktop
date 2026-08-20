use std::sync::Arc;
use std::time::Duration;

use ora_logging::{ora_error, ora_info, ora_warn};
use ora_process::ManagedProcess;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use crate::PluginRuntimeError;
use crate::codec::{read_frame, write_frame};
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

/// Reads plugin registration, notifications, and responses from the stdout protocol stream.
pub(crate) async fn run_reader<R>(mut stdout: R, inner: Arc<RuntimeInner>)
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

/// Drains plugin stderr continuously so logging cannot block the child process.
pub(crate) async fn run_stderr<R>(mut stderr: R, plugin_id: String)
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
pub(crate) async fn run_supervisor<P>(
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
    inner.exited_tx.send_replace(true);
}
