//! The turn process fence: before a session outlives a turn that ended by cancel or failure, the
//! processes that turn started in sessions of their own are killed and shown gone.
//!
//! A tool that backgrounds a job from a shell that called `setsid` leaves it in no group the
//! Node signals, and the Agent reports the tool `completed` while the job keeps writing the
//! checkout. The attempt mark reaches such a job wherever it moved, so the fence narrows the
//! attempt's own [`ora_utils::process::MarkScope`] to the turn: processes that started at or
//! after the moment the prompt was admitted, outside every session that a process of the attempt
//! from before that moment still belongs to. The Agent's own processes, and what it starts in
//! their sessions during the turn, such as a language server it launches lazily, keep running;
//! the turn's tool jobs do not. Killing and the evidence are those of a session stop: freeze and
//! kill through pinned handles, then two clean scans in a row, on the Node's one stop worker.
//!
//! An exempt process can still start a job in a new session after the fence came back clean,
//! while the quiet window runs. A scan-only re-check of the same scope just before the stop
//! settles `stopped` catches it; it kills nothing, so what it finds ends the session, whose stop
//! then covers the whole attempt.
//!
//! The turn's start lives in memory only. A Node that restarts mid-turn ends the session as
//! `interrupted`, and restart recovery stops the whole attempt, so nothing about a turn needs to
//! survive a restart.
use super::{FIRST_RECHECK, files, marks, next_recheck, spawner::SessionAttemptSpawner};
use crate::managed::attempt_lock::StopEvidence;
use ora_node_db::AttemptId;
use ora_node_protocol::ExecutionId;
use ora_utils::process::MarkScope;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// How long the re-check before a `stopped` settlement may take, waiting for the worker included.
/// It only scans, so it needs far less than the fence; a worker held past it reads as Unknown.
pub(in crate::session) const RECHECK_BUDGET: Duration = Duration::from_secs(1);

/// Reads the boot clock in the ticks process start times use.
type Clock = fn() -> std::io::Result<u64>;

/// Where one session's turns start and how their processes are stopped.
#[derive(Clone)]
pub(in crate::session) struct TurnProcesses {
    spawner: SessionAttemptSpawner,
    clock: Clock,
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
    /// Two clean scans in a row.
    Stopped,
    /// The turn's processes still ran, or nothing showed that they are gone; `evidence` is the
    /// last observation.
    Unstopped { evidence: StopEvidence },
}

/// What one observation on the worker found.
enum Round {
    Observed(StopEvidence),
    /// The attempt's mark scope record is missing or unreadable.
    NoScope,
}

impl TurnProcesses {
    pub(in crate::session) fn new(spawner: SessionAttemptSpawner) -> Self {
        Self::with_clock(spawner, ora_utils::process::linux_boot_ticks)
    }

    /// Uses `clock` in place of the boot clock, so a test can make it fail.
    pub(in crate::session) fn with_clock(spawner: SessionAttemptSpawner, clock: Clock) -> Self {
        Self { spawner, clock }
    }

    /// Records the start of the turn whose prompt is being admitted now. Every process the
    /// prompt makes the Agent start, starts later.
    pub(in crate::session) fn start(&self) -> TurnStart {
        match (self.clock)() {
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
    /// the new Agent's own, so it is never reported stopped and nothing is signalled; the session
    /// then ends, and its stop covers both attempts.
    pub(in crate::session) async fn stop(
        &self,
        execution: &ExecutionId,
        start: Option<&TurnStart>,
        bound: Duration,
    ) -> TurnStop {
        let Some((attempt, since_ticks)) = self.recorded(execution, start) else {
            return UNKNOWN;
        };
        let deadline = tokio::time::Instant::now() + bound;
        let evidence = observe_until(deadline, || {
            let (files, attempt) = (self.spawner.files(), attempt.clone());
            async move {
                // Signal outcomes prove nothing; only the scans after the kill decide.
                let round = files::off_runtime(files::budget_until(deadline), move || {
                    turn_scope(&files, &attempt, since_ticks).map_or(Round::NoScope, |scope| {
                        let _ = scope.kill();
                        Round::Observed(marks::evidence(|| scope.scan()))
                    })
                })
                .await;
                match round {
                    Some(Round::Observed(evidence)) => Some(evidence),
                    Some(Round::NoScope) => {
                        ora_logging::ora_warn!(execution_id = %execution, "the stopped turn's attempt has no readable mark scope");
                        None
                    }
                    None => Some(StopEvidence::Unknown),
                }
            }
        })
        .await;
        conclude(execution, "fence", evidence)
    }

    /// Scans the turn's processes once more, killing nothing, just before the stop settles.
    pub(in crate::session) async fn recheck(
        &self,
        execution: &ExecutionId,
        start: Option<&TurnStart>,
    ) -> TurnStop {
        let Some((attempt, since_ticks)) = self.recorded(execution, start) else {
            return UNKNOWN;
        };
        let files = self.spawner.files();
        let round = files::off_runtime(RECHECK_BUDGET, move || {
            turn_scope(&files, &attempt, since_ticks).map_or(Round::NoScope, |scope| {
                Round::Observed(marks::evidence(|| scope.scan()))
            })
        })
        .await;
        let evidence = match round {
            Some(Round::Observed(evidence)) => evidence,
            Some(Round::NoScope) => {
                ora_logging::ora_warn!(execution_id = %execution, "the stopped turn's attempt has no readable mark scope");
                StopEvidence::Unknown
            }
            None => StopEvidence::Unknown,
        };
        conclude(execution, "re-check", evidence)
    }

    /// The recorded start of a turn whose attempt still runs; `None`, logged, otherwise.
    fn recorded(
        &self,
        execution: &ExecutionId,
        start: Option<&TurnStart>,
    ) -> Option<(AttemptId, u64)> {
        let (attempt, since_ticks) = match start {
            Some(TurnStart::Recorded {
                attempt,
                since_ticks,
            }) => (attempt, *since_ticks),
            Some(TurnStart::Unrecorded) | None => {
                ora_logging::ora_warn!(execution_id = %execution, "the stopped turn has no recorded start");
                return None;
            }
        };
        if self.spawner.identity().attempt != *attempt {
            ora_logging::ora_warn!(execution_id = %execution, attempt = %attempt, "the stopped turn's attempt was replaced during the turn");
            return None;
        }
        Some((attempt.clone(), since_ticks))
    }
}

/// What the fence reports when it cannot tell the turn's processes apart.
const UNKNOWN: TurnStop = TurnStop::Unstopped {
    evidence: StopEvidence::Unknown,
};

/// The attempt's mark scope narrowed to one turn, read from its record on the worker.
fn turn_scope(
    files: &files::StopFiles,
    attempt: &AttemptId,
    since_ticks: u64,
) -> Option<MarkScope> {
    match files.mark_scope(attempt) {
        Ok(Some(scope)) => Some(scope.new_sessions_since(since_ticks)),
        Ok(None) | Err(_) => None,
    }
}

/// Turns the last observation into the outcome, logging one that is not clean.
fn conclude(execution: &ExecutionId, stage: &str, evidence: StopEvidence) -> TurnStop {
    if evidence == StopEvidence::Stopped {
        return TurnStop::Stopped;
    }
    ora_logging::ora_warn!(execution_id = %execution, stage, evidence = ?evidence, "the processes of a stopped turn were not shown gone");
    TurnStop::Unstopped { evidence }
}

/// Observes until one observation is clean or another one would not fit before `deadline`,
/// waiting the Node's shared doubling backoff in between; returns the last evidence. An
/// observation that yields `None` cannot succeed by repetition and ends the wait as Unknown.
pub(in crate::session) async fn observe_until<F, Fut>(
    deadline: tokio::time::Instant,
    mut observe: F,
) -> StopEvidence
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<StopEvidence>>,
{
    let mut pause = FIRST_RECHECK;
    loop {
        let Some(evidence) = observe().await else {
            return StopEvidence::Unknown;
        };
        if evidence == StopEvidence::Stopped || tokio::time::Instant::now() + pause >= deadline {
            return evidence;
        }
        tokio::time::sleep(pause).await;
        pause = next_recheck(pause);
    }
}
