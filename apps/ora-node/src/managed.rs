//! Node-owned Git execution: every command is a direct child of the Node, and a command with
//! side effects is only settled by the stop evidence of its attempt lock.
mod attempt_lock;
#[cfg(test)]
mod attempt_lock_tests;
mod delivery;
mod exec;
#[cfg(test)]
mod exec_tests;
mod repository;
mod restore;
#[cfg(test)]
mod restore_tests;
pub(crate) use attempt_lock::AttemptLocks;
pub(crate) use delivery::AsNode;
pub(crate) use repository::{AttemptSettlement, DirectGit, RunAs};
pub(crate) use restore::{RestoreContext, RestoreStops, StopCheck};

use crate::git::ExecutionGitRunner;
use attempt_lock::{LockDurability, StopEvidence};
use exec::{Capture, Invocation, StopGrace};
use gitlancer::{GitCommand, GitExecError, GitIntent, GitOutput, GitRunner};
use ora_node_db::{
    AttemptCommand, AttemptId, DurableWrites, Execution, ProcessAttempt, ProcessJournal, WriteGuard,
};
use ora_node_protocol::{ExecutionId, WorktreeFailure, WorktreeFailureCode};
use ora_process::ProcessIdentity;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// The most output one command may return; a caller's own limit can only be smaller.
const CAPTURE_LIMIT: usize = 1_048_576;

/// Deployment supplies paths and environment explicitly; none select a directory from HOME.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    /// Cloud's workload identity; management retains its protected OS identity.
    #[serde(default)]
    pub workload_uid: Option<u32>,
    /// The state directory of the retired process host. Accepted so configurations written for
    /// the process host still load; other configured directories must still stay clear of it.
    #[serde(default)]
    pub host_directory: Option<PathBuf>,
    /// Accepted so configurations written for the process host still load; no longer used.
    #[serde(default)]
    pub expected_uid: Option<u32>,
    /// The `ora-reaper` executable; when absent, the one installed next to this program.
    #[serde(default)]
    pub reaper_program: Option<PathBuf>,
    pub git_program: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub command_timeout_ms: u64,
    /// Clone/fetch/push use a separate bounded budget; older deployments receive ten minutes.
    #[serde(default = "default_network_timeout_ms")]
    pub network_timeout_ms: u64,
    pub cleanup_timeout_ms: u64,
    pub shutdown_grace_ms: u64,
}

impl ProcessConfig {
    /// Selects budgets using Git's declared intent rather than guessing command-line strings.
    fn command_budget(&self, intent: GitIntent) -> Duration {
        Duration::from_millis(if intent == GitIntent::Network {
            self.network_timeout_ms
        } else {
            self.command_timeout_ms
        })
    }

    /// The budget of one command: its own time limit when it names one, else its intent's.
    fn budget_for(&self, command: &GitCommand) -> Duration {
        command
            .time_limit
            .unwrap_or_else(|| self.command_budget(command.intent))
    }

    /// The OS identity Git takes between fork and exec. The group equals the user, with no
    /// supplementary groups, no capabilities and `no_new_privs`.
    fn identity(&self, run_as: RunAs) -> std::io::Result<ProcessIdentity> {
        match (run_as, self.workload_uid) {
            (RunAs::Workload, Some(uid)) => Ok(ProcessIdentity::Linux(
                ora_utils::process::LinuxChildIdentity::new(uid, uid)?,
            )),
            (RunAs::Workload, None) | (RunAs::Node, _) => Ok(ProcessIdentity::Inherit),
        }
    }

    /// Where the reaper is installed for this deployment.
    pub fn reaper(&self) -> std::io::Result<PathBuf> {
        match &self.reaper_program {
            Some(program) => Ok(program.clone()),
            None => Ok(std::env::current_exe()?.with_file_name("ora-reaper")),
        }
    }
}

/// Keeps network commands bounded when reading a pre-existing deployment configuration.
fn default_network_timeout_ms() -> u64 {
    600_000
}

/// Shutdown stops admission immediately while allowing the current command a bounded grace period.
#[derive(Clone, Default)]
pub struct Shutdown(Arc<Mutex<Stop>>);

#[derive(Default)]
struct Stop {
    requested: Option<Instant>,
    failure: Option<String>,
}

impl Shutdown {
    /// May be called by a signal task without borrowing or interrupting the Node's SQLite transaction.
    pub fn request(&self) {
        self.stop().requested.get_or_insert_with(Instant::now);
    }
    /// Stops the Node because it lost something it cannot run safely without. The first reason
    /// is kept, and the process reports it instead of a clean exit.
    pub fn fail(&self, reason: &str) {
        let mut stop = self.stop();
        stop.requested.get_or_insert_with(Instant::now);
        stop.failure.get_or_insert_with(|| reason.to_owned());
    }
    /// The reason of a stop the Node forced on itself, if there was one.
    pub fn failure(&self) -> Option<String> {
        self.stop().failure.clone()
    }
    /// Reports admission closure; this is not evidence that a process has stopped.
    pub fn requested(&self) -> bool {
        self.stop().requested.is_some()
    }
    /// Bounds graceful completion from the first request; repeated signals never extend the deadline.
    fn expired(&self, grace: Duration) -> bool {
        self.stop()
            .requested
            .is_some_and(|at| at.elapsed() >= grace)
    }
    fn stop(&self) -> std::sync::MutexGuard<'_, Stop> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Runs every Git command directly; only mutations need durable business-execution associations.
pub struct ManagedGitRunner<W: WriteGuard = DurableWrites> {
    journal: ProcessJournal<W>,
    config: ProcessConfig,
    execution: RefCell<Option<ExecutionId>>,
    shutdown: Shutdown,
    git: DirectGit,
}

impl<W: WriteGuard> ManagedGitRunner<W> {
    /// Shares the Node database lease, checks deployment inputs and settles what the previous
    /// Node process left behind, before any business recovery can start a command.
    pub(crate) fn new(
        journal: ProcessJournal<W>,
        config: ProcessConfig,
        shutdown: Shutdown,
        home: &Path,
    ) -> std::io::Result<Self> {
        if !config.git_program.is_absolute()
            || config
                .reaper_program
                .as_ref()
                .is_some_and(|p| !p.is_absolute())
            || config
                .host_directory
                .as_ref()
                .is_some_and(|p| !p.is_absolute())
            || config.command_timeout_ms == 0
            || config.network_timeout_ms == 0
            || config.network_timeout_ms > 1_200_000
            || config.cleanup_timeout_ms == 0
        {
            return Err(std::io::Error::other(
                "process paths must be absolute, deadlines positive, and network budget at most twenty minutes",
            ));
        }
        if let Some(uid) = config.workload_uid {
            // SAFETY: reading the management identity changes no process credentials.
            if unsafe { libc::geteuid() } != 0 || uid == 0 || !config.environment.is_empty() {
                return Err(std::io::Error::other(
                    "isolated workloads require root management, a non-root workload identity and an empty base environment",
                ));
            }
        }
        let locks = AttemptLocks::open(home)?;
        let runner = Self {
            git: DirectGit::new(config.clone(), shutdown.clone(), locks)?,
            journal,
            config,
            shutdown,
            execution: RefCell::new(None),
        };
        runner.settle_leftovers()?;
        Ok(runner)
    }

    /// Goes through the lock files of the previous Node process before anything else runs.
    ///
    /// A lock a recorded attempt still owes evidence for is left to that attempt's own recovery,
    /// which reads the result; its process group is signalled here so the evidence is ready.
    /// Every other lock belongs to a command the database never recorded, or to one already
    /// settled: once free it is deleted, and while held it blocks new unrecorded commands.
    fn settle_leftovers(&self) -> std::io::Result<()> {
        let unsettled: BTreeSet<AttemptId> = self
            .journal
            .unsettled()
            .map_err(std::io::Error::other)?
            .into_iter()
            .collect();
        let cleanup = Duration::from_millis(self.config.cleanup_timeout_ms);
        for (attempt, cwd) in self.git.locks.entries()? {
            if unsettled.contains(&attempt) {
                self.git.locks.terminate(&attempt);
                continue;
            }
            match self
                .git
                .runtime
                .block_on(self.git.locks.await_stop(&attempt, cleanup))
            {
                StopEvidence::Stopped => self.git.locks.retire(&attempt),
                StopEvidence::Running | StopEvidence::Unknown => {
                    ora_logging::ora_warn!(attempt = %attempt, directory = %cwd.display(), "a Git command of an earlier Node process has not stopped");
                    self.git.locks.keep_leftover(attempt, cwd);
                }
            }
        }
        Ok(())
    }

    /// Obtains stop evidence for every old attempt before allowing Git to interpret the resource as a residual.
    fn handoff(&self, execution: &ExecutionId) -> std::io::Result<()> {
        for attempt in self
            .journal
            .pending(execution)
            .map_err(std::io::Error::other)?
        {
            self.settle(&attempt.id)?;
        }
        Ok(())
    }

    /// Retires a recorded attempt once its lock proves nothing of it is left.
    fn settle(&self, attempt: &AttemptId) -> std::io::Result<()> {
        let evidence = self.git.runtime.block_on(self.git.locks.await_stop(
            attempt,
            Duration::from_millis(self.config.cleanup_timeout_ms),
        ));
        if evidence != StopEvidence::Stopped {
            return Err(std::io::Error::other(
                "stop evidence of an earlier Git command is unverified",
            ));
        }
        self.journal
            .cleaned(attempt)
            .map_err(std::io::Error::other)?;
        self.git.locks.retire(attempt);
        Ok(())
    }
}

impl<W: WriteGuard> ExecutionGitRunner for ManagedGitRunner<W> {
    /// Settles all uncertain mutation attempts without inventing terminal business results.
    fn shutdown(&self) -> Result<(), WorktreeFailure> {
        self.shutdown.request();
        let cleanup = || -> std::io::Result<()> {
            for execution in self
                .journal
                .pending_executions()
                .map_err(std::io::Error::other)?
            {
                self.handoff(&execution)?;
            }
            Ok(())
        };
        cleanup().map_err(|error| {
            crate::resources::failure(WorktreeFailureCode::ResultUnknown, error.to_string())
        })
    }
    /// Graceful shutdown closes new-command admission without discarding historical results.
    fn accepting_work(&self) -> bool {
        !self.shutdown.requested()
    }
    /// Legacy in-flight direct Git has no cleanup evidence and cannot silently become managed work.
    fn begin_execution(&self, record: &Execution) -> Result<(), WorktreeFailure> {
        let result = self
            .journal
            .manage(record)
            .map_err(std::io::Error::other)
            .and_then(|()| self.handoff(record.command.execution_id()));
        result.map_err(|e| {
            crate::resources::failure(WorktreeFailureCode::ResultUnknown, e.to_string())
        })?;
        *self.execution.borrow_mut() = Some(record.command.execution_id().clone());
        Ok(())
    }
    /// Recorded attempts remain in the journal even when execution returns an error or is cancelled.
    fn end_execution(&self) {
        self.execution.borrow_mut().take();
    }
}

impl<W: WriteGuard> GitRunner for ManagedGitRunner<W> {
    /// Captures a bounded prefix; truncation is an error and never fed to a Git fact parser.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        self.run_bounded(command, CAPTURE_LIMIT, CAPTURE_LIMIT)
    }

    /// Requires durable execution context for mutation; uncertain attempts block subsequent observations.
    fn run_bounded(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
    ) -> Result<GitOutput, GitExecError> {
        let fail = |source| GitExecError::OutputReadFailed {
            stream: "managed execution",
            source,
        };
        let execution = self.execution.borrow().clone();
        if let Some(execution) = &execution
            && !self
                .journal
                .pending(execution)
                .map_err(std::io::Error::other)
                .map_err(fail)?
                .is_empty()
        {
            return Err(fail(std::io::Error::other(
                "stop evidence of an earlier Git command is unverified",
            )));
        }
        if self.shutdown.requested() {
            return Err(fail(std::io::Error::other("Node is stopping")));
        }
        let mut environment: Vec<_> = self
            .config
            .environment
            .iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        environment.push((
            "GIT_TERMINAL_PROMPT".into(),
            if command.env.terminal_prompt {
                "1"
            } else {
                "0"
            }
            .into(),
        ));
        environment.push(("LANG".into(), command.env.lang.clone().into()));
        environment.push(("GIT_PAGER".into(), command.env.pager.clone().into()));
        environment.extend(
            command
                .env
                .variables
                .iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        let invocation = |lock| Invocation {
            command,
            environment: environment.clone(),
            identity: RunAs::Workload,
            capture: Capture::Bounded {
                stdout: stdout.min(CAPTURE_LIMIT),
                stderr: stderr.min(CAPTURE_LIMIT),
            },
            deadline: Instant::now() + self.config.budget_for(command),
            stop_grace: StopGrace::Honored,
            before_cut: None,
            lock,
        };
        if command.intent == GitIntent::ReadOnly {
            // Read-only preflight has no accepted business execution yet. What it leaves behind
            // cannot write, so it runs without an attempt lock or a record.
            let finished = self.git.runtime.block_on(exec::run(
                &self.config,
                &self.git.locks,
                &self.shutdown,
                invocation(None),
            ));
            return normalize(command, finished.output.map_err(fail)?);
        }
        let execution = execution
            .ok_or_else(|| fail(std::io::Error::other("mutation without accepted execution")))?;
        let attempt = ProcessAttempt {
            execution,
            id: AttemptId::generate(),
        };
        // The lock file exists before the record, and the record before the dispatch.
        self.git
            .locks
            .create(&attempt.id, &command.cwd, LockDurability::Recorded)
            .map_err(fail)?;
        self.journal
            .record(
                &attempt,
                &AttemptCommand {
                    cwd: command.cwd.clone(),
                    args: command.args.clone(),
                },
            )
            .map_err(std::io::Error::other)
            .map_err(fail)?;
        let finished = self.git.runtime.block_on(exec::run(
            &self.config,
            &self.git.locks,
            &self.shutdown,
            invocation(Some(&attempt.id)),
        ));
        // Even an output failure may follow a successful mutation. Only stop evidence retires it.
        if finished.settlement == AttemptSettlement::Unverified {
            return Err(fail(std::io::Error::other(
                "Git stop evidence is unverified",
            )));
        }
        self.journal
            .cleaned(&attempt.id)
            .map_err(std::io::Error::other)
            .map_err(fail)?;
        self.git.locks.retire(&attempt.id);
        normalize(command, finished.output.map_err(fail)?)
    }
}

/// Normalizes managed output once for both fact queries and business mutations.
fn normalize(command: &GitCommand, output: GitOutput) -> Result<GitOutput, GitExecError> {
    if output.code == Some(0) {
        Ok(output)
    } else {
        Err(GitExecError::NonZeroExit {
            code: output.code,
            args: command.args.clone(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}
