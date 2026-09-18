//! Node-owned Git execution association; process facts remain owned by host and guardian.
mod repository;
mod transport;
pub(crate) use repository::CloneRecovery;

use crate::git::ExecutionGitRunner;
use gitlancer::{GitCommand, GitExecError, GitIntent, GitOutput, GitRunner};
use ora_node_db::{DurableWrites, Execution, ProcessAttempt, ProcessJournal, WriteGuard};
use ora_node_protocol::{ExecutionId, WorktreeFailure, WorktreeFailureCode};
use ora_process_client::ProcessHost;
use ora_process_protocol::*;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Deployment supplies paths and environment explicitly; none select a directory from HOME.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    pub host_directory: PathBuf,
    pub expected_uid: u32,
    pub git_program: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub command_timeout_ms: u64,
    pub cleanup_timeout_ms: u64,
    pub shutdown_grace_ms: u64,
}

/// Shutdown stops admission immediately while allowing the current command a bounded grace period.
#[derive(Clone, Default)]
pub struct Shutdown(Arc<Mutex<Option<Instant>>>);
impl Shutdown {
    /// May be called by a signal task without borrowing or interrupting the Node's SQLite transaction.
    pub fn request(&self) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_or_insert_with(Instant::now);
    }
    /// Reports admission closure; this is not evidence that a process has stopped.
    pub fn requested(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }
    /// Bounds graceful completion from the first request; repeated signals never extend the deadline.
    fn expired(&self, grace: Duration) -> bool {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some_and(|at| at.elapsed() >= grace)
    }
}

/// All Git commands use the host; only mutations need durable business-execution associations.
pub struct ManagedGitRunner<W: WriteGuard = DurableWrites> {
    journal: ProcessJournal<W>,
    config: ProcessConfig,
    execution: RefCell<Option<ExecutionId>>,
    runtime: tokio::runtime::Runtime,
    shutdown: Shutdown,
    owner: RunLifetime,
    reads: RefCell<Option<(ScopeId, usize)>>,
}

impl<W: WriteGuard> ManagedGitRunner<W> {
    /// Shares the Node database lease and checks deployment inputs before any managed launch.
    pub(crate) fn new(
        journal: ProcessJournal<W>,
        config: ProcessConfig,
        shutdown: Shutdown,
    ) -> std::io::Result<Self> {
        if !config.host_directory.is_absolute()
            || !config.git_program.is_absolute()
            || config.command_timeout_ms == 0
            || config.cleanup_timeout_ms == 0
        {
            return Err(std::io::Error::other(
                "process paths must be absolute and deadlines positive",
            ));
        }
        let stat = ora_utils::process::linux_process(std::process::id())?;
        Ok(Self {
            journal,
            config,
            shutdown,
            execution: RefCell::new(None),
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
            owner: RunLifetime::TerminateOnOwnerExit {
                pid: stat.pid,
                start_ticks: stat.start_ticks,
            },
            reads: RefCell::new(None),
        })
    }

    /// Stops and seals every old attempt before allowing Git to interpret the resource as a residual.
    fn handoff(&self, execution: &ExecutionId) -> std::io::Result<()> {
        for attempt in self
            .journal
            .pending(execution)
            .map_err(std::io::Error::other)?
        {
            self.runtime.block_on(transport::close(
                &ProcessHost::new(attempt.host_directory.clone(), attempt.expected_uid),
                attempt.intent.scope,
                Duration::from_millis(self.config.cleanup_timeout_ms),
            ))?;
            self.journal
                .cleaned(attempt.intent.run)
                .map_err(std::io::Error::other)?;
        }
        Ok(())
    }
}

impl<W: WriteGuard> ExecutionGitRunner for ManagedGitRunner<W> {
    /// Closes read-only scopes and all uncertain mutation scopes without inventing terminal business results.
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
            if let Some((scope, _)) = *self.reads.borrow() {
                let client =
                    ProcessHost::new(self.config.host_directory.clone(), self.config.expected_uid);
                self.runtime.block_on(transport::close(
                    &client,
                    scope,
                    Duration::from_millis(self.config.cleanup_timeout_ms),
                ))?;
            }
            self.reads.borrow_mut().take();
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
    /// Remote attempts remain in the journal even when execution returns an error or is cancelled.
    fn end_execution(&self) {
        self.execution.borrow_mut().take();
    }
}

impl<W: WriteGuard> GitRunner for ManagedGitRunner<W> {
    /// Captures a bounded prefix; truncation is an error and never fed to a Git fact parser.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        self.run_bounded(command, GUARDIAN_CAPTURE_LIMIT, GUARDIAN_CAPTURE_LIMIT)
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
                "prior Run cleanup is unverified",
            )));
        }
        if self.shutdown.requested() {
            return Err(fail(std::io::Error::other("Node is stopping")));
        }
        let mut spec = RunSpec::new(
            self.config.git_program.as_os_str(),
            &command.cwd,
            DescendantPolicy::Cleanup {
                grace: Duration::ZERO,
            },
        );
        spec.args = command.args.iter().map(OsString::from).collect();
        spec.env = self
            .config
            .environment
            .iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        spec.env.insert(
            "GIT_TERMINAL_PROMPT".into(),
            if command.env.terminal_prompt {
                "1"
            } else {
                "0"
            }
            .into(),
        );
        spec.env
            .insert("LANG".into(), command.env.lang.clone().into());
        spec.env
            .insert("GIT_PAGER".into(), command.env.pager.clone().into());
        spec.env.extend(
            command
                .env
                .variables
                .iter()
                .map(|(k, v)| (k.into(), v.into())),
        );
        spec.output = OutputPolicy::Capture {
            stdout_limit: stdout.min(GUARDIAN_CAPTURE_LIMIT),
            stderr_limit: stderr.min(GUARDIAN_CAPTURE_LIMIT),
        };
        spec.lifetime = self.owner;
        let client = ProcessHost::new(self.config.host_directory.clone(), self.config.expected_uid);
        if command.intent == GitIntent::ReadOnly {
            // Read-only preflight has no accepted business execution yet. Its host-owned attempts
            // share a bounded Scope; they never authorize resource mutation or recovery.
            let mut reads = self.reads.borrow_mut();
            if let Some((scope, count)) = *reads
                && count >= GUARDIAN_RUN_LIMIT
            {
                self.runtime
                    .block_on(transport::close(
                        &client,
                        scope,
                        Duration::from_millis(self.config.cleanup_timeout_ms),
                    ))
                    .map_err(fail)?;
                *reads = None;
            }
            let (scope, count) = reads.get_or_insert_with(|| (ScopeId::new(), 0));
            *count += 1;
            let intent = HostRunIntent {
                scope: *scope,
                run: RunId::new(),
                spec,
                host_disconnect: GuardianHostDisconnect::KeepRunning,
            };
            let result = self.runtime.block_on(transport::execute(
                &client,
                &intent,
                &self.config,
                &self.shutdown,
            ));
            if result.is_err() {
                self.runtime
                    .block_on(transport::close(
                        &client,
                        *scope,
                        Duration::from_millis(self.config.cleanup_timeout_ms),
                    ))
                    .map_err(fail)?;
                *reads = None;
            }
            return normalize(command, result.map_err(fail)?);
        }
        let execution = execution
            .ok_or_else(|| fail(std::io::Error::other("mutation without accepted execution")))?;
        let attempt = ProcessAttempt {
            execution,
            host_directory: self.config.host_directory.clone(),
            expected_uid: self.config.expected_uid,
            intent: HostRunIntent {
                scope: ScopeId::new(),
                run: RunId::new(),
                spec,
                host_disconnect: GuardianHostDisconnect::KeepRunning,
            },
        };
        self.journal
            .record(&attempt)
            .map_err(std::io::Error::other)
            .map_err(fail)?;
        let result = self.runtime.block_on(transport::execute(
            &client,
            &attempt.intent,
            &self.config,
            &self.shutdown,
        ));
        // Even an output failure may follow a successful mutation. Only closed Scope evidence retires it.
        self.runtime
            .block_on(transport::close(
                &client,
                attempt.intent.scope,
                Duration::from_millis(self.config.cleanup_timeout_ms),
            ))
            .map_err(fail)?;
        self.journal
            .cleaned(attempt.intent.run)
            .map_err(std::io::Error::other)
            .map_err(fail)?;
        normalize(command, result.map_err(fail)?)
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
