//! Original restore identities and stop evidence; timeouts never release checkout responsibility.
use super::{
    DirectGit, ProcessConfig,
    attempt_lock::{AttemptLocks, StopEvidence},
};
use ora_node_db::{AttemptId, RestoreAttempt, SessionJournal};
use ora_node_protocol::ExecutionId;
use std::{io, time::Duration};

/// Expiry ends actor polling; explicit handover can still obtain late original evidence.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum StopCheck {
    WithinBound,
    Handover,
}

/// One shared durable authority for live restore, crash handover and late execution queries.
#[derive(Clone)]
pub(crate) struct RestoreStops {
    pub(crate) journal: SessionJournal,
    config: ProcessConfig,
    bound: Duration,
    locks: AttemptLocks,
}

impl RestoreStops {
    pub(crate) fn new(
        journal: SessionJournal,
        config: ProcessConfig,
        bound: Duration,
        locks: AttemptLocks,
    ) -> Self {
        Self {
            journal,
            config,
            bound,
            locks,
        }
    }

    /// Only local wall timestamps are persisted; no monotonic clock is reconstructed after restart.
    pub(crate) fn now_ms() -> i64 {
        (ora_logging::clock::now_local().unix_timestamp_nanos() / 1_000_000) as i64
    }

    /// Cancellation is a session fact even when no command is active at that instant.
    pub(crate) fn request_session(&self, execution: &ExecutionId) -> io::Result<()> {
        self.journal
            .request_session_restore_stop(execution, Self::now_ms(), self.bound.as_millis() as i64)
            .map_err(io::Error::other)
    }

    /// Anchors once before the stop request, including recovery from a crash before the
    /// original anchor.
    pub(crate) fn request(&self, attempt: &AttemptId) -> io::Result<()> {
        self.journal
            .request_restore_stop(attempt, Self::now_ms(), self.bound.as_millis() as i64)
            .map_err(io::Error::other)
    }

    /// Checks only original identities. Expired attempts are queried only by an explicit handover.
    pub(crate) async fn poll(&self, execution: &ExecutionId, check: StopCheck) -> io::Result<bool> {
        let attempts = self
            .journal
            .pending_restore_attempts(execution)
            .map_err(io::Error::other)?;
        for attempt in attempts {
            let expired = attempt
                .stop_bound_at_ms
                .is_some_and(|at| Self::now_ms() >= at);
            if expired && check == StopCheck::WithinBound {
                self.journal
                    .mark_restore_stop_unconfirmed(&attempt.process.id, Self::now_ms())
                    .map_err(io::Error::other)?;
                continue;
            }
            // A failed timestamp write cannot prevent the safety-critical stop request. It also
            // cannot manufacture a cleaned association or a new origin on the next retry.
            if let Err(error) = self.request(&attempt.process.id) {
                self.locks.terminate(&attempt.process.id);
                return Err(error);
            }
            // The anchor write itself may consume the remaining bound. Re-read its durable
            // timestamp rather than granting a fresh cleanup budget to an initially missing clock.
            let current = self
                .journal
                .pending_restore_attempts(execution)
                .map_err(io::Error::other)?
                .into_iter()
                .find(|current| current.process.id == attempt.process.id)
                .ok_or_else(|| io::Error::other("original restore attempt disappeared"))?;
            let bound_at = current
                .stop_bound_at_ms
                .ok_or_else(|| io::Error::other("restore stop bound is missing"))?;
            let remaining = bound_at.saturating_sub(Self::now_ms());
            if remaining <= 0 {
                self.journal
                    .mark_restore_stop_unconfirmed(&attempt.process.id, Self::now_ms())
                    .map_err(io::Error::other)?;
                if check == StopCheck::WithinBound {
                    continue;
                }
            }
            let budget = match check {
                StopCheck::WithinBound => (remaining as u64).min(self.config.cleanup_timeout_ms),
                StopCheck::Handover => self.config.cleanup_timeout_ms,
            };
            let proof =
                tokio::time::timeout(Duration::from_millis(budget), self.prove(&attempt)).await;
            if matches!(proof, Ok(true)) {
                self.journal
                    .confirm_restore_stop(&attempt.process.id)
                    .map_err(io::Error::other)?;
                // The evidence is durable now; the lock file has nothing left to prove.
                self.locks.retire(&attempt.process.id);
            } else {
                self.journal
                    .mark_restore_stop_unconfirmed(&attempt.process.id, Self::now_ms())
                    .map_err(io::Error::other)?;
            }
        }
        Ok(self
            .journal
            .pending_restore_attempts(execution)
            .map_err(io::Error::other)?
            .is_empty())
    }

    /// Whether an original command of this restore may still be running, so the inputs it reads
    /// stay on disk. An unreadable journal keeps them too: missing evidence is never permission.
    pub(crate) fn retains_inputs(&self, execution: &ExecutionId) -> bool {
        !self
            .journal
            .pending_restore_attempts(execution)
            .map(|pending| pending.is_empty())
            .unwrap_or(/*default*/ false)
    }

    /// Ends what the original command's clue names and asks its lock whether the tree is gone.
    ///
    /// A lock that can be taken is the whole proof: whether the command exited, was killed or
    /// never started, nothing of it can still write. A missing lock file is no proof.
    async fn prove(&self, attempt: &RestoreAttempt) -> bool {
        self.locks
            .await_stop(
                &attempt.process.id,
                Duration::from_millis(self.config.cleanup_timeout_ms),
            )
            .await
            == StopEvidence::Stopped
    }

    /// Waits only until the persisted first-request bound, then transfers responsibility intact.
    pub(crate) async fn wait(&self, execution: &ExecutionId) -> bool {
        loop {
            match self.poll(execution, StopCheck::WithinBound).await {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => {
                    ora_logging::ora_warn!(execution_id = %execution, error = %error, "restore stop evidence remains unknown");
                    return false;
                }
            }
            let Ok(attempts) = self.journal.pending_restore_attempts(execution) else {
                return false;
            };
            if attempts.iter().any(|attempt| attempt.stop_unconfirmed) {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(/*millis*/ 100)).await;
        }
    }
}

/// The runner is serialized by the restore's Git mutex; cancellation is observed at admission.
pub(crate) struct RestoreContext {
    pub(crate) stops: RestoreStops,
    pub(crate) execution: ExecutionId,
    pub(crate) cancellation: crate::session::RestoreCancel,
}

impl DirectGit {
    /// Associates the next restore's commands without sharing mutable runner state with delivery.
    pub(crate) fn track_restore(&mut self, stops: RestoreStops) {
        self.restore_stops = Some(stops);
    }
}
