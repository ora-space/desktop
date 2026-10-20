//! Revision delivery's Git runs through the host like clone's, as separate guarded Runs.
//!
//! Delivery Git only reads the checkout, writes scratch files in its Git directory and moves an
//! `refs/ora/revisions/` ref, so its Runs are not journaled as business mutations: a delivery
//! interrupted before freezing prepares again from scratch, and the guardian ends a Run whose
//! Node exited. Each Run gets its own Scope, closed before the next command starts.
use super::{CloneHost, transport};
use crate::revision::RestoreGitRunner;
use gitlancer::{GitCommand, GitExecError, GitOutput, GitRunner};
use ora_process_client::ProcessHost;
use ora_process_protocol::*;
use std::time::{Duration, Instant};

/// A restore caps execution only; delivery and cleanup keep their independent configured budgets.
enum Budget {
    Command,
    Restore(Instant),
}

impl GitRunner for CloneHost {
    /// Captures both streams up to the guardian limit.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        self.run_bounded(command, GUARDIAN_CAPTURE_LIMIT, GUARDIAN_CAPTURE_LIMIT)
    }

    /// Runs one command in a fresh Scope and closes the Scope before returning, so no delivery
    /// Git can still be writing when the next step reads the result.
    fn run_bounded(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
    ) -> Result<GitOutput, GitExecError> {
        self.capture(command, stdout, stderr, Budget::Command)
    }
}

impl RestoreGitRunner for CloneHost {
    /// Caps the host exchange as well as process polling without clipping Scope cleanup.
    fn run_bounded_before(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
        deadline: Instant,
    ) -> Result<GitOutput, GitExecError> {
        self.capture(command, stdout, stderr, Budget::Restore(deadline))
    }
}

impl CloneHost {
    /// Shares supervision and cleanup between ordinary delivery and deadline-capped restore.
    fn capture(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
        budget: Budget,
    ) -> Result<GitOutput, GitExecError> {
        let fail = |source| GitExecError::OutputReadFailed {
            stream: "delivery Git",
            source,
        };
        if self.shutdown.requested() {
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
        let client = ProcessHost::new(self.config.host_directory.clone(), self.config.expected_uid);
        let intent = HostRunIntent {
            scope: ScopeId::new(),
            run: RunId::new(),
            spec: self.spec(
                command,
                OutputPolicy::Capture {
                    stdout_limit: stdout.min(GUARDIAN_CAPTURE_LIMIT),
                    stderr_limit: stderr.min(GUARDIAN_CAPTURE_LIMIT),
                },
            ),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        };
        let result = match budget {
            Budget::Command => self.runtime.block_on(transport::execute(
                &client,
                &intent,
                command.intent,
                &self.config,
                &self.shutdown,
            )),
            Budget::Restore(deadline) => self.runtime.block_on(transport::execute_before(
                &client,
                &intent,
                command.intent,
                &self.config,
                &self.shutdown,
                deadline,
            )),
        };
        self.runtime
            .block_on(transport::close(
                &client,
                intent.scope,
                Duration::from_millis(self.config.cleanup_timeout_ms),
            ))
            .map_err(fail)?;
        let output = result.map_err(fail)?;
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
