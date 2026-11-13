//! Restore commands carry one absolute cutoff through the supervised execution boundary.
use gitlancer::{GitCommand, GitExecError, GitOutput, GitRunner};
use std::{io, time::Instant};

/// Executes restore Git with a caller-owned cutoff, retaining the runner's stop/cleanup barrier.
/// Implementations cap their command budget and request closure on expiry; returning never
/// substitutes a timeout or a close request for the runner's normal cleanup evidence.
pub(crate) trait RestoreGitRunner: GitRunner {
    /// Production binds every command to the original session before any Git side effect.
    fn bind_restore(
        &mut self,
        _execution: &ora_node_protocol::ExecutionId,
        _cancellation: crate::session::RestoreCancel,
    ) {
    }

    /// Admission fences command-boundary cancellation, including local staging between commands.
    fn check_admission(&self) -> Result<(), GitExecError> {
        Ok(())
    }

    /// Runs under min(the command's own budget, the remaining restore deadline).
    fn run_bounded_before(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
        deadline: Instant,
    ) -> Result<GitOutput, GitExecError>;
}

/// A borrowed runner view prevents a new command after expiry without changing delivery's policy.
pub(super) struct RestoreCommands<'a, R> {
    pub(super) runner: &'a R,
    pub(super) deadline: Instant,
}

impl<R: RestoreGitRunner> RestoreGitRunner for RestoreCommands<'_, R> {
    /// Local staging uses the same cancellation and deadline fence as Git.
    fn check_admission(&self) -> Result<(), GitExecError> {
        check(self.deadline)?;
        self.runner.check_admission()
    }
    /// Keeps nested Git steps on the same deadline and evidence owner.
    fn run_bounded_before(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
        deadline: Instant,
    ) -> Result<GitOutput, GitExecError> {
        self.runner
            .run_bounded_before(command, stdout, stderr, deadline.min(self.deadline))
    }
}

impl<R: RestoreGitRunner> GitRunner for RestoreCommands<'_, R> {
    /// Retains the runner's capture ceiling for the unbounded trait entry point.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        self.run_bounded(command, usize::MAX, usize::MAX)
    }

    /// Checks admission; the supervised runner owns execution expiry and the separate close barrier.
    fn run_bounded(
        &self,
        command: &GitCommand,
        stdout: usize,
        stderr: usize,
    ) -> Result<GitOutput, GitExecError> {
        self.check_admission()?;
        self.runner
            .run_bounded_before(command, stdout, stderr, self.deadline)
    }
}

/// A deadline refusal is not Git's permanent missing-object verdict.
pub(super) fn check(deadline: Instant) -> Result<(), GitExecError> {
    if Instant::now() >= deadline {
        Err(GitExecError::OutputReadFailed {
            stream: "Revision restore",
            source: io::Error::new(
                io::ErrorKind::TimedOut,
                "restore execution deadline expired",
            ),
        })
    } else {
        Ok(())
    }
}
