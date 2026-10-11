//! Reconciles authenticated model-ending intent with the durable command's exact terminal reason.

#[cfg(test)]
mod tests;

use super::ports::{CommandSettlement, SessionLedger};
use super::queue::{Plan, plan};
use ora_node_protocol::{EndSessionReason, ExecutionId};
use std::path::Path;
use std::time::Duration;
use tokio::sync::{Notify, watch};

/// The controller may be reconnecting, but no model authority is retained during this bound.
pub(super) const END_RECONCILE_WAIT: Duration = Duration::from_secs(/*secs*/ 30);

/// Waits only for the already-durable end command; unrelated revocations never enter this path.
pub(super) async fn reconcile<L: SessionLedger>(
    ledger: &L,
    execution: &ExecutionId,
    wake: &Notify,
    mut stopping: watch::Receiver<bool>,
    wait: Duration,
) -> Result<EndSessionReason, &'static str> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        // Register before reading: command receipt between the read and await cannot lose a wake.
        let notified = wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        match ledger.queued_commands(execution).map(plan) {
            Ok(Plan::End {
                command_id,
                reason,
                discarded,
            }) => {
                for command in discarded {
                    ledger
                        .settle_command(execution, &command, CommandSettlement::Discarded)
                        .map_err(|_| "ledger_unavailable")?;
                }
                ledger
                    .settle_command(execution, &command_id, CommandSettlement::Executed)
                    .map_err(|_| "ledger_unavailable")?;
                return Ok(reason);
            }
            Ok(Plan::Turn { .. } | Plan::Wait) => {}
            Err(_) => return Err("ledger_unavailable"),
        }
        // The last read also catches a persisted command whose acknowledgement lost its wake.
        if tokio::time::Instant::now() >= deadline {
            return Err("model_access_revoked");
        }
        tokio::select! {
            biased;
            _ = stopping.wait_for(|stop| *stop) => return Ok(EndSessionReason::Cancelled),
            _ = tokio::time::sleep_until(deadline) => {},
            _ = notified => {}
        }
    }
}

/// An unstarted session has no transcript, but delivery still requires a sealed empty JSONL.
pub(super) fn seal_unstarted_history(
    root: &Path,
    execution: &ExecutionId,
) -> Result<(), &'static str> {
    let path =
        ora_history::history_path(root, execution.as_str()).map_err(|_| "history_unavailable")?;
    let Some(parent) = path.parent() else {
        return Err("history_unavailable");
    };
    ora_utils::path::create_directories_without_symlinks(parent)
        .map_err(|_| "history_unavailable")?;
    match std::fs::OpenOptions::new()
        .create_new(/*create_new*/ true)
        .write(/*write*/ true)
        .open(&path)
    {
        Ok(file) => file.sync_all().map_err(|_| "history_unavailable"),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(path).map_err(|_| "history_unavailable")?;
            if metadata.is_file() && !metadata.file_type().is_symlink() {
                Ok(())
            } else {
                Err("history_unavailable")
            }
        }
        Err(_) => Err("history_unavailable"),
    }
}
