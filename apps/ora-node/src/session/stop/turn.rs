//! The turn process fence: before a session outlives a turn that ended by cancel or failure, the
//! processes that turn started in sessions of their own are killed and shown gone.
//!
//! A tool that backgrounds a job from a shell that called `setsid` leaves it in no group the
//! Node signals, and the Agent reports the tool `completed` while the job keeps writing the
//! checkout. The attempt mark reaches such a job wherever it moved, so the fence narrows the
//! attempt's own [`MarkScope`] to the turn: processes that started at or after the moment the
//! prompt was admitted, outside every session that a process of the attempt from before that
//! moment still belongs to. The Agent's own processes, and what it starts in their sessions
//! during the turn, such as a language server it launches lazily, keep running; the turn's tool
//! jobs do not. Killing and the evidence are those of a session stop: freeze and kill through
//! pinned handles, then two clean scans in a row, on the Node's one stop worker.
//!
//! The turn's start lives in memory only. A Node that restarts mid-turn ends the session as
//! `interrupted`, and restart recovery stops the whole attempt, so nothing about a turn needs to
//! survive a restart.
use super::{FIRST_RECHECK, files, marks, next_recheck, spawner::SessionAttemptSpawner};
use crate::managed::attempt_lock::StopEvidence;
use ora_node_db::AttemptId;
use ora_node_protocol::ExecutionId;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Where one session's turns start and how their processes are stopped.
#[derive(Clone)]
pub(in crate::session) struct TurnProcesses {
    spawner: SessionAttemptSpawner,
}

/// When and in which attempt one turn's prompt was admitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::session) enum TurnStart {
    Recorded {
        attempt: AttemptId,
        since_ticks: u64,
    },
    /// The boot clock could not be read, so nothing tells the turn's processes from earlier ones.
    Unrecorded,
}

/// Filled by the admission commit of one prompt, just before the prompt is written.
pub(in crate::session) type TurnStartCell = Arc<OnceLock<TurnStart>>;

/// What the fence showed about a stopped turn's processes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::session) enum TurnStop {
    /// Two clean scans in a row after the kill.
    Stopped,
    /// The bound passed with the turn's processes still running, or with nothing to show they
    /// are gone; `evidence` is the last observation.
    Unstopped { evidence: StopEvidence },
}

impl TurnProcesses {
    pub(in crate::session) fn new(spawner: SessionAttemptSpawner) -> Self {
        Self { spawner }
    }

    /// Records the start of the turn whose prompt is being admitted now. Every process the
    /// prompt makes the Agent start, starts later.
    pub(in crate::session) fn start(&self) -> TurnStart {
        match ora_utils::process::linux_boot_ticks() {
            Ok(since_ticks) => TurnStart::Recorded {
                attempt: self.spawner.identity().attempt,
                since_ticks,
            },
            Err(_) => TurnStart::Unrecorded,
        }
    }

    /// Kills the turn's processes and waits, at most `bound`, for evidence that they are gone.
    ///
    /// An attempt replaced during the turn leaves no scope that tells this turn's processes from
    /// the new Agent's own, so it is never reported stopped; the session then ends, and its stop
    /// covers both attempts.
    pub(in crate::session) async fn stop(
        &self,
        execution: &ExecutionId,
        start: Option<&TurnStart>,
        bound: Duration,
    ) -> TurnStop {
        let unknown = TurnStop::Unstopped {
            evidence: StopEvidence::Unknown,
        };
        let Some(TurnStart::Recorded {
            attempt,
            since_ticks,
        }) = start
        else {
            return unknown;
        };
        if &self.spawner.identity().attempt != attempt {
            return unknown;
        }
        let scope = match self.spawner.files().mark_scope(attempt) {
            Ok(Some(scope)) => scope.new_sessions_since(*since_ticks),
            Ok(None) | Err(_) => return unknown,
        };
        let deadline = tokio::time::Instant::now() + bound;
        let mut pause = FIRST_RECHECK;
        loop {
            let round = scope.clone();
            // Signal outcomes prove nothing; only the scans after the kill decide.
            let evidence = files::off_runtime(files::budget_until(deadline), move || {
                let _ = round.kill();
                marks::evidence(|| round.scan())
            })
            .await
            .unwrap_or(StopEvidence::Unknown);
            if evidence == StopEvidence::Stopped {
                return TurnStop::Stopped;
            }
            if tokio::time::Instant::now() + pause >= deadline {
                ora_logging::ora_warn!(execution_id = %execution, evidence = ?evidence, "the processes of a stopped turn were not shown gone");
                return TurnStop::Unstopped { evidence };
            }
            tokio::time::sleep(pause).await;
            pause = next_recheck(pause);
        }
    }
}
