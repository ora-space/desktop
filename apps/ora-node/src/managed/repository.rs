use super::*;
use ora_node_db::{AttemptEnding, CloneExecution};
use std::ffi::OsString;

/// How a clone attempt ended, concluded only once its stop evidence held.
///
/// The live and recovery paths share this classification so the same evidence always yields the
/// same business outcome, whichever process happens to read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttemptSettlement {
    /// Git exited with its own verdict.
    Exited(i32),
    /// A signal ended Git before it reached a verdict (Node stop, command deadline or an external
    /// kill), so no write can follow and success is impossible.
    Terminated(i32),
    /// Nothing of the attempt is running, but nobody collected an exit status: a restarted Node
    /// found it stopped, or it never started. Success is impossible here too.
    Stopped,
    /// The stop evidence does not hold or is missing; nothing may be concluded about the directory.
    Unverified,
}

/// Which OS identity a Git command runs as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunAs {
    /// The workload user, for every command that touches a checkout.
    Workload,
    /// The Node's own identity, for Git Server access from a transport repository.
    Node,
}

/// One thread's means of running Git directly: dispatching, settling and inspecting commands.
///
/// It holds no Node database connection, so it can run away from the thread that owns SQLite.
/// Every attempt it settles was recorded, with its lock file, before it was handed over, which
/// lets a restart settle the same attempt without anything this value remembered.
pub(crate) struct DirectGit {
    pub(super) config: ProcessConfig,
    pub(super) restore_stops: Option<super::RestoreStops>,
    pub(super) restore_context: Option<super::RestoreContext>,
    pub(super) shutdown: Shutdown,
    pub(super) locks: AttemptLocks,
    pub(super) runtime: tokio::runtime::Runtime,
}

impl DirectGit {
    /// Each instance owns its runtime, so one can move to another thread while the runner keeps its own.
    pub(super) fn new(
        config: ProcessConfig,
        shutdown: Shutdown,
        locks: AttemptLocks,
    ) -> std::io::Result<Self> {
        Ok(Self {
            config,
            restore_stops: None,
            restore_context: None,
            shutdown,
            locks,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        })
    }

    /// Whether the Node began stopping; long waits between Git Server attempts end early on it.
    pub(crate) fn stopping(&self) -> bool {
        self.shutdown.requested()
    }

    /// The workload user Git that touches checkouts runs as, if the deployment separates it.
    pub(crate) fn workload_uid(&self) -> Option<u32> {
        self.config.workload_uid
    }

    /// The lock directory, for whoever settles this Node's restore attempts.
    pub(crate) fn locks(&self) -> &AttemptLocks {
        &self.locks
    }

    /// Directories in which an unrecorded command of an earlier pass may still be writing.
    pub(crate) fn leftover_directories(&self) -> Vec<PathBuf> {
        self.locks.leftover_directories()
    }

    /// Starts a recorded attempt and settles it once Git ends or its deadline passes.
    pub(crate) fn dispatch(
        &self,
        attempt: &ProcessAttempt,
        command: &GitCommand,
    ) -> AttemptSettlement {
        self.runtime
            .block_on(exec::run(
                &self.config,
                &self.locks,
                &self.shutdown,
                Invocation {
                    command,
                    environment: clone_environment(command),
                    identity: RunAs::Workload,
                    capture: Capture::Discard,
                    deadline: Instant::now() + self.config.command_budget(GitIntent::Network),
                    stop_grace: StopGrace::Honored,
                    before_cut: None,
                    lock: Some(&attempt.id),
                },
            ))
            .settlement
    }

    /// Settles an attempt an earlier process dispatched: ends what its clue names, then asks the
    /// lock. Its exit status is gone with the process that started it.
    ///
    /// The evidence comes first: no directory fact is read while the old Git could still write.
    pub(crate) fn settle(&self, attempt: &ProcessAttempt) -> AttemptSettlement {
        match self.runtime.block_on(self.locks.await_stop(
            &attempt.id,
            Duration::from_millis(self.config.cleanup_timeout_ms),
        )) {
            StopEvidence::Stopped => AttemptSettlement::Stopped,
            StopEvidence::Running | StopEvidence::Unknown => AttemptSettlement::Unverified,
        }
    }

    /// Inspects local facts under the same non-secret deployment environment.
    pub(crate) fn inspect(&self, command: &GitCommand) -> std::io::Result<GitOutput> {
        self.runtime
            .block_on(exec::run(
                &self.config,
                &self.locks,
                &self.shutdown,
                Invocation {
                    command,
                    environment: clone_environment(command),
                    identity: RunAs::Workload,
                    capture: Capture::Bounded {
                        stdout: 8192,
                        stderr: 0,
                    },
                    deadline: Instant::now() + self.config.budget_for(command),
                    stop_grace: StopGrace::Honored,
                    before_cut: None,
                    lock: None,
                },
            ))
            .output
    }
}

/// Intentionally does not copy the Worktree deployment environment, which may contain secrets.
pub(super) fn clone_environment(command: &GitCommand) -> Vec<(OsString, OsString)> {
    command
        .env
        .variables
        .iter()
        .map(|(key, value)| (key.into(), value.into()))
        .chain([
            ("LANG".into(), "C".into()),
            ("GIT_PAGER".into(), "cat".into()),
        ])
        .collect()
}

impl<W: WriteGuard> ManagedGitRunner<W> {
    /// Exposes deployment paths for clone's stricter non-overlapping root checks.
    pub(crate) fn process_config(&self) -> &ProcessConfig {
        &self.config
    }

    /// The executor this runner uses when a caller drives a clone on its own thread.
    pub(crate) fn direct_git(&self) -> &DirectGit {
        &self.git
    }

    /// A separate executor with the same deployment, locks and stop signal, for another thread to own.
    pub(crate) fn detached_git(&self) -> std::io::Result<DirectGit> {
        DirectGit::new(
            self.config.clone(),
            self.shutdown.clone(),
            self.git.locks.clone(),
        )
    }

    /// Establishes managed responsibility while the directory-created phase still proves no dispatch.
    pub(crate) fn prepare_clone(&self, record: &CloneExecution) -> Result<(), ora_node_db::Error> {
        self.journal.manage_clone(record)
    }

    /// Lists the attempts already recorded for the execution; recovery settles these, never new ones.
    pub(crate) fn clone_attempts(
        &self,
        record: &CloneExecution,
    ) -> std::io::Result<Vec<ProcessAttempt>> {
        self.journal
            .attempts(&record.command.execution_id)
            .map_err(std::io::Error::other)
    }

    /// The ending an earlier pass or process already recorded for the attempt, if any. A settled
    /// attempt is never probed again: its lock file may be gone by now.
    pub(crate) fn recorded_settlement(
        &self,
        attempt: &ProcessAttempt,
    ) -> std::io::Result<Option<AttemptSettlement>> {
        Ok(self
            .journal
            .ending(&attempt.id)
            .map_err(std::io::Error::other)?
            .map(|ending| match ending {
                AttemptEnding::Exited(code) => AttemptSettlement::Exited(code),
                AttemptEnding::Terminated(signal) => AttemptSettlement::Terminated(signal),
                AttemptEnding::Stopped => AttemptSettlement::Stopped,
            }))
    }

    /// Creates the attempt's lock file, then persists the attempt; only a recorded attempt may
    /// be dispatched.
    pub(crate) fn record_clone_attempt(
        &self,
        record: &CloneExecution,
        command: &GitCommand,
    ) -> std::io::Result<ProcessAttempt> {
        if self.shutdown.requested() {
            return Err(std::io::Error::other("Node is stopping"));
        }
        let attempt = ProcessAttempt {
            execution: record.command.execution_id.clone(),
            id: AttemptId::generate(),
        };
        self.git
            .locks
            .create(&attempt.id, &command.cwd, LockDurability::Recorded)?;
        let recorded = self.journal.record(
            &attempt,
            &AttemptCommand {
                cwd: command.cwd.clone(),
                args: command.args.clone(),
            },
        );
        if let Err(error) = recorded {
            // No record refers to the lock file and nothing was started with it.
            self.git.locks.retire(&attempt.id);
            return Err(std::io::Error::other(error));
        }
        Ok(attempt)
    }

    /// Records how the attempt ended, then retires it and its lock file.
    ///
    /// The ending is persisted before the attempt is marked settled, so the database can later
    /// prove which business result the single attempt supports.
    pub(crate) fn persist_settlement(
        &self,
        attempt: &ProcessAttempt,
        settlement: AttemptSettlement,
    ) -> std::io::Result<()> {
        let ending = match settlement {
            AttemptSettlement::Exited(code) => AttemptEnding::Exited(code),
            AttemptSettlement::Terminated(signal) => AttemptEnding::Terminated(signal),
            AttemptSettlement::Stopped => AttemptEnding::Stopped,
            // Without stop evidence the attempt keeps its lock file and stays unsettled.
            AttemptSettlement::Unverified => return Ok(()),
        };
        self.journal
            .record_ending(&attempt.id, ending)
            .map_err(std::io::Error::other)?;
        self.journal
            .cleaned(&attempt.id)
            .map_err(std::io::Error::other)?;
        self.git.locks.retire(&attempt.id);
        Ok(())
    }
}
