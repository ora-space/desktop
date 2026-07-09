use crate::PluginManagerError;
use ora_process::ManagedProcess;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time;

/// Carries the collected output from one plugin process invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginProcessOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) exit_code: Option<i32>,
}

/// Drives one spawned plugin process through stdin, stdout, stderr, wait, and timeout handling.
pub(crate) async fn collect_plugin_process_output<Process>(
    plugin_id: &str,
    process: &mut Process,
    stdin: String,
    timeout: Duration,
) -> Result<PluginProcessOutput, PluginManagerError>
where
    Process: ManagedProcess,
{
    match time::timeout(timeout, drive_plugin_process(plugin_id, process, stdin)).await {
        Ok(output) => output,
        Err(_) => {
            let _ = process.kill().await;
            Err(PluginManagerError::ProcessTimedOut {
                plugin_id: plugin_id.to_string(),
            })
        }
    }
}

/// Runs all process IO futures together so a noisy stderr pipe cannot block stdout handling.
async fn drive_plugin_process<Process>(
    plugin_id: &str,
    process: &mut Process,
    stdin: String,
) -> Result<PluginProcessOutput, PluginManagerError>
where
    Process: ManagedProcess,
{
    let stdin_pipe = process
        .take_stdin()
        .ok_or_else(|| missing_pipe_error(plugin_id, "stdin"))?;
    let stdout_pipe = process
        .take_stdout()
        .ok_or_else(|| missing_pipe_error(plugin_id, "stdout"))?;
    let stderr_pipe = process
        .take_stderr()
        .ok_or_else(|| missing_pipe_error(plugin_id, "stderr"))?;

    let (_, stdout, stderr, exit_code) = tokio::try_join!(
        write_stdin(plugin_id, stdin_pipe, stdin),
        read_stream(plugin_id, stdout_pipe, "stdout"),
        read_stream(plugin_id, stderr_pipe, "stderr"),
        wait_for_exit(plugin_id, process),
    )?;

    Ok(PluginProcessOutput {
        stdout,
        stderr,
        exit_code,
    })
}

/// Writes the full newline-framed JSON-RPC request and then closes plugin stdin.
async fn write_stdin<Stdin>(
    plugin_id: &str,
    mut stdin_pipe: Stdin,
    stdin: String,
) -> Result<(), PluginManagerError>
where
    Stdin: AsyncWrite + Unpin,
{
    stdin_pipe
        .write_all(stdin.as_bytes())
        .await
        .map_err(|error| process_io_error(plugin_id, "write stdin", error))?;
    stdin_pipe
        .shutdown()
        .await
        .map_err(|error| process_io_error(plugin_id, "close stdin", error))
}

/// Reads one text stream to completion so JSON-RPC parsing can stay outside process plumbing.
async fn read_stream<Stream>(
    plugin_id: &str,
    mut stream: Stream,
    stream_name: &str,
) -> Result<String, PluginManagerError>
where
    Stream: AsyncRead + Unpin,
{
    let mut output = String::new();
    stream
        .read_to_string(&mut output)
        .await
        .map_err(|error| process_io_error(plugin_id, &format!("read {stream_name}"), error))?;

    Ok(output)
}

/// Waits for the plugin process and preserves platforms that do not expose a numeric exit code.
async fn wait_for_exit<Process>(
    plugin_id: &str,
    process: &Process,
) -> Result<Option<i32>, PluginManagerError>
where
    Process: ManagedProcess,
{
    process
        .wait()
        .await
        .map(|status| status.code())
        .map_err(|error| process_io_error(plugin_id, "wait for process exit", error))
}

/// Builds a stable error when the process backend did not expose one required pipe.
fn missing_pipe_error(plugin_id: &str, pipe_name: &str) -> PluginManagerError {
    PluginManagerError::ProcessFailed {
        plugin_id: plugin_id.to_string(),
        message: format!("process did not expose {pipe_name} pipe"),
    }
}

/// Preserves process IO context while hiding backend-specific error types from callers.
fn process_io_error(plugin_id: &str, operation: &str, error: std::io::Error) -> PluginManagerError {
    PluginManagerError::ProcessFailed {
        plugin_id: plugin_id.to_string(),
        message: format!("failed to {operation}: {error}"),
    }
}
