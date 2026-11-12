//! Revision delivery's and `ora-git`'s Git, and a restore's, as directly run commands.
//!
//! Restore Git records original identities in the session journal before admission.
//! Delivery Git only reads the checkout, writes scratch files in its Git directory and moves an
//! `refs/ora/revisions/` ref, so its commands are not journaled as business mutations: a delivery
//! interrupted before freezing prepares again from scratch. Each still inherits an attempt lock,
//! because a push that outlived a killed Node must be known to have stopped before the next one
//! starts. Read-only commands need neither.
use super::{
    AttemptSettlement, CAPTURE_LIMIT, DirectGit, RunAs,
    attempt_lock::LockDurability,
    exec::{self, Capture, Invocation, StopGrace},
    repository::clone_environment,
};
use crate::revision::RestoreGitRunner;
use gitlancer::{GitCommand, GitExecError, GitIntent, GitOutput, GitRunner};
use ora_node_db::{AttemptCommand, AttemptId, ProcessAttempt};
use std::time::Instant;

/// A restore caps execution only; delivery and cleanup keep their independent configured budgets.
enum Budget {
    Command,
    Restore(Instant),
}

/// The same executor under the Node identity, for Git Server access from transport repositories;
/// it borrows or owns its executor.
pub(crate) struct AsNode<H>(pub(crate) H);

impl<H: std::borrow::Borrow<DirectGit>> GitRunner for AsNode<H> {
    /// Captures both streams up to the capture limit.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        self.run_bounded(command, CAPTURE_LIMIT, CAPTURE_LIMIT)
    }

    /// Runs one command without the workload isolation.
    fn run_bounded(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
    ) -> Result<GitOutput, GitExecError> {
        self.0
            .borrow()
            .run_scoped(command, stdout, stderr, RunAs::Node, Budget::Command)
    }
}

impl<H: std::borrow::BorrowMut<DirectGit>> RestoreGitRunner for AsNode<H> {
    /// A restore's Node-identity commands belong to the original session like the workload's.
    fn bind_restore(
        &mut self,
        execution: &ora_node_protocol::ExecutionId,
        cancellation: crate::session::RestoreCancel,
    ) {
        self.0.borrow_mut().bind_restore(execution, cancellation);
    }

    /// Admission fences Git Server, transport and `commit_of` commands between steps.
    fn check_admission(&self) -> Result<(), GitExecError> {
        self.0.borrow().check_admission()
    }

    /// Runs within the restore's absolute cutoff, with the same stop evidence as the workload's
    /// commands, so a slow Git Server fetch cannot outlive it.
    fn run_bounded_before(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
        deadline: Instant,
    ) -> Result<GitOutput, GitExecError> {
        self.0.borrow().run_scoped(
            command,
            stdout,
            stderr,
            RunAs::Node,
            Budget::Restore(deadline),
        )
    }
}

impl GitRunner for DirectGit {
    /// Captures both streams up to the capture limit.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        self.run_bounded(command, CAPTURE_LIMIT, CAPTURE_LIMIT)
    }

    /// Runs one command and returns once its stop evidence held, so no delivery Git can still
    /// be writing when the next step reads the result.
    fn run_bounded(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
    ) -> Result<GitOutput, GitExecError> {
        self.run_scoped(command, stdout, stderr, RunAs::Workload, Budget::Command)
    }
}

impl RestoreGitRunner for DirectGit {
    /// The restore mutex provides exclusive ownership while binding the command context.
    fn bind_restore(
        &mut self,
        execution: &ora_node_protocol::ExecutionId,
        cancellation: crate::session::RestoreCancel,
    ) {
        self.restore_context = self
            .restore_stops
            .clone()
            .map(|stops| super::RestoreContext {
                stops,
                execution: execution.clone(),
                cancellation,
            });
    }

    /// An admitted command finishes under its budget; cancellation fences the next side effect.
    fn check_admission(&self) -> Result<(), GitExecError> {
        if self.shutdown.requested()
            || self.restore_context.as_ref().is_some_and(|context| {
                context.cancellation.requested()
                    || !context
                        .stops
                        .journal
                        .restore_stop_requested(&context.execution)
                        .is_ok_and(|requested| !requested)
            })
        {
            Err(GitExecError::OutputReadFailed {
                stream: "Revision restore",
                source: std::io::Error::other("restore cancellation requested"),
            })
        } else {
            Ok(())
        }
    }

    /// Caps the command at the restore's cutoff without clipping the wait for its stop evidence.
    fn run_bounded_before(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
        deadline: Instant,
    ) -> Result<GitOutput, GitExecError> {
        self.run_scoped(
            command,
            stdout,
            stderr,
            RunAs::Workload,
            Budget::Restore(deadline),
        )
    }
}

impl DirectGit {
    /// Runs one command as `identity` and settles it before returning.
    fn run_scoped(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
        identity: RunAs,
        budget: Budget,
    ) -> Result<GitOutput, GitExecError> {
        let fail = |source| GitExecError::OutputReadFailed {
            stream: "delivery Git",
            source,
        };
        if self.shutdown.requested()
            || self.restore_context.as_ref().is_some_and(|context| {
                context.cancellation.requested()
                    || !context
                        .stops
                        .journal
                        .restore_stop_requested(&context.execution)
                        .is_ok_and(|requested| !requested)
            })
        {
            return Err(fail(std::io::Error::other("Node is stopping")));
        }
        if let Budget::Restore(deadline) = budget
            && Instant::now() >= deadline
        {
            return Err(fail(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "restore execution deadline expired",
            )));
        }
        let command_deadline = Instant::now() + self.config.budget_for(command);
        let (deadline, stop_grace) = match budget {
            Budget::Command => (command_deadline, StopGrace::Honored),
            Budget::Restore(deadline) => (deadline.min(command_deadline), StopGrace::Ignored),
        };
        let invocation = |lock, before_cut| Invocation {
            command,
            environment: clone_environment(command),
            identity,
            capture: Capture::Bounded {
                stdout: stdout.min(CAPTURE_LIMIT),
                stderr: stderr.min(CAPTURE_LIMIT),
            },
            deadline,
            stop_grace,
            before_cut,
            lock,
        };
        let run = |invocation| {
            self.runtime.block_on(exec::run(
                &self.config,
                &self.locks,
                &self.shutdown,
                invocation,
            ))
        };
        let attempt = AttemptId::generate();
        let output = if let Some(context) = &self.restore_context {
            // A restore records every command before it is dispatched: lock file, then record.
            self.locks
                .create(&attempt, &command.cwd, LockDurability::Recorded)
                .map_err(fail)?;
            let recorded = context.stops.journal.record_restore_attempt(
                &ProcessAttempt {
                    execution: context.execution.clone(),
                    id: attempt.clone(),
                },
                &AttemptCommand {
                    cwd: command.cwd.clone(),
                    args: command.args.clone(),
                },
                ora_node_protocol::StopContainment::BestEffort,
            );
            if let Err(error) = recorded {
                // No record refers to the lock file and nothing was started with it.
                self.locks.retire(&attempt);
                return Err(fail(std::io::Error::other(error)));
            }
            // The stop clock is anchored before the command is cut, never after the kill.
            let anchor = || {
                let _ = context.stops.request(&attempt);
            };
            let finished = run(invocation(Some(&attempt), Some(&anchor)));
            if !self
                .runtime
                .block_on(
                    context
                        .stops
                        .poll(&context.execution, super::StopCheck::WithinBound),
                )
                .map_err(fail)?
            {
                return Err(fail(std::io::Error::other("restore stop is unconfirmed")));
            }
            finished.output
        } else if command.intent == GitIntent::ReadOnly {
            run(invocation(None, None)).output
        } else {
            // Nothing durable names this command, so the lock is all a restarted Node has. While
            // an earlier unrecorded command may still be writing, none starts.
            if !self.locks.leftover_directories().is_empty() {
                return Err(fail(std::io::Error::other(
                    "an earlier Git command has not stopped",
                )));
            }
            self.locks
                .create(&attempt, &command.cwd, LockDurability::Unrecorded)
                .map_err(fail)?;
            let finished = run(invocation(Some(&attempt), None));
            if finished.settlement == AttemptSettlement::Unverified {
                self.locks.keep_leftover(attempt, command.cwd.clone());
            } else {
                self.locks.retire(&attempt);
            }
            finished.output
        };
        let output = output.map_err(fail)?;
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
}
