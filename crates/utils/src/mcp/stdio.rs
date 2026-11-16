//! Stdio MCP probe: spawn, newline JSON-RPC handshake, then reclaim the process.

use super::protocol::{
    JsonRpcResponse, MAX_RESPONSE_BYTES, ProbeError, accept_result, decode_line, encode_line,
    initialize_request, initialized_notification, tools_list_request,
};
use crate::process::hide_console_window;
use std::path::PathBuf;
use std::process::Stdio as StdStdio;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{Instant, timeout, timeout_at};

/// Spawns `command`, completes initialize + tools/list, then closes stdin and waits for exit.
///
/// The child handle stays owned by this function until it has been reaped: cancelling the
/// handshake at the hard timeout still runs the explicit kill-and-wait teardown, so a probe can
/// never leak an orphaned server process.
pub async fn probe_stdio(
    command: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: Option<PathBuf>,
    overall_timeout: Duration,
) -> Result<(), ProbeError> {
    let deadline = Instant::now() + overall_timeout;
    let mut std_command = std::process::Command::new(&command);
    std_command
        .args(&args)
        .stdin(StdStdio::piped())
        .stdout(StdStdio::piped())
        // stderr carries server diagnostics at best; it is never read, and a full pipe must not
        // block the handshake, so it is discarded rather than piped.
        .stderr(StdStdio::null());
    if let Some(cwd) = &cwd {
        std_command.current_dir(cwd);
    }
    for (key, value) in &env {
        std_command.env(key, value);
    }
    hide_console_window(&mut std_command);

    let mut command = Command::from(std_command);
    // Backstop only: normal teardown below is explicit, but a dropped future must still kill.
    command.kill_on_drop(true);
    let mut child = command.spawn().map_err(|_| ProbeError::SpawnFailed)?;

    let mut stdin = child.stdin.take().ok_or(ProbeError::SpawnFailed)?;
    let stdout = child.stdout.take().ok_or(ProbeError::SpawnFailed)?;
    let mut reader = BufReader::new(stdout);

    let result = match timeout_at(deadline, handshake(&mut stdin, &mut reader)).await {
        Ok(result) => result,
        Err(_) => Err(ProbeError::Timeout),
    };

    // Teardown: closing stdin lets a well-behaved server exit on its own; anything still alive
    // after the deadline is killed, and `wait` confirms the process is actually reaped.
    drop(stdin);
    reclaim_child(&mut child, deadline).await;
    result
}

/// Runs initialize, the initialized notification, and tools/list over the child's pipes.
pub async fn handshake<W: AsyncWrite + Unpin, R: AsyncBufRead + Unpin>(
    stdin: &mut W,
    reader: &mut R,
) -> Result<(), ProbeError> {
    let mut remaining = MAX_RESPONSE_BYTES;
    send_request(
        stdin,
        reader,
        initialize_request(1),
        Some(1),
        &mut remaining,
    )
    .await?;
    send_request(
        stdin,
        reader,
        initialized_notification(),
        None,
        &mut remaining,
    )
    .await?;
    match send_request(
        stdin,
        reader,
        tools_list_request(2),
        Some(2),
        &mut remaining,
    )
    .await
    {
        Ok(_) => Ok(()),
        // A completed initialize followed by any tools/list protocol failure means the tools
        // surface is unavailable, not that the handshake failed.
        Err(ProbeError::ExitedPrematurely | ProbeError::HandshakeFailed) => {
            Err(ProbeError::ToolsUnavailable)
        }
        Err(error) => Err(error),
    }
}

/// Writes one framed message and, when `expected_id` is set, reads until a matching result.
async fn send_request<W: AsyncWrite + Unpin, R: AsyncBufRead + Unpin>(
    stdin: &mut W,
    reader: &mut R,
    message: serde_json::Value,
    expected_id: Option<u64>,
    remaining: &mut usize,
) -> Result<Option<JsonRpcResponse>, ProbeError> {
    let bytes = encode_line(&message)?;
    stdin
        .write_all(&bytes)
        .await
        .map_err(|_| ProbeError::ExitedPrematurely)?;
    stdin
        .flush()
        .await
        .map_err(|_| ProbeError::ExitedPrematurely)?;
    let Some(expected_id) = expected_id else {
        return Ok(None);
    };
    loop {
        let line = read_bounded_line(reader, remaining).await?;
        if line.is_empty() {
            // EOF: the server closed stdout (or died) before answering the handshake.
            return Err(ProbeError::ExitedPrematurely);
        }
        // Servers sometimes print non-JSON diagnostics on stdout; skip noise lines and keep
        // waiting for a JSON-RPC frame until the caller's deadline.
        let Ok(response) = decode_line(&line) else {
            continue;
        };
        if accept_result(&response, expected_id) {
            return Ok(Some(response));
        }
        // Notifications and server requests carry no matching id; an answered but mismatched or
        // failed response means the peer is not completing this handshake.
        if response.id.is_some() {
            return Err(ProbeError::HandshakeFailed);
        }
    }
}

/// Counts noise and protocol frames together, including unterminated lines, before allocation.
async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    remaining: &mut usize,
) -> Result<String, ProbeError> {
    let mut bytes = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|_| ProbeError::ExitedPrematurely)?;
        if available.is_empty() {
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        if count > *remaining {
            return Err(ProbeError::HandshakeFailed);
        }
        bytes.extend_from_slice(&available[..count]);
        *remaining -= count;
        reader.consume(count);
        if newline.is_some() {
            break;
        }
    }
    String::from_utf8(bytes).map_err(|_| ProbeError::HandshakeFailed)
}

/// Closes the child cleanly within the remaining deadline, escalating to kill if needed.
async fn reclaim_child(child: &mut Child, deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() && timeout(remaining, child.wait()).await.is_ok() {
        return;
    }
    // `kill` awaits the exit internally, so returning from here means the process is reaped.
    let _ = child.kill().await;
    let _ = child.wait().await;
}
