use std::process::Command;
use std::time::Instant;

use crate::error::GitExecError;
use crate::exec::command::GitCommand;
use crate::exec::output::GitOutput;

/// Executes one prepared Git command and returns normalized process output.
pub trait GitRunner {
    /// Runs a fully assembled Git command so use-case layers stay independent from process details.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError>;
}

/// Executes Git commands through the system `git` binary.
#[derive(Debug, Default, Clone, Copy)]
pub struct CliGitRunner;

impl GitRunner for CliGitRunner {
    /// Spawns the Git CLI with stable automation defaults so upper layers can trust execution semantics.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        let started_at = Instant::now();
        let mut process = Command::new("git");
        process.current_dir(&command.cwd);
        process.args(&command.args);

        // The execution contract disables prompts and paging so agent-driven flows stay deterministic.
        process.env(
            "GIT_TERMINAL_PROMPT",
            if command.env.terminal_prompt {
                "1"
            } else {
                "0"
            },
        );
        process.env("LANG", &command.env.lang);
        process.env("GIT_PAGER", &command.env.pager);

        let output = process.output().map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                GitExecError::GitNotFound
            } else {
                GitExecError::SpawnFailed {
                    args: command.args.clone(),
                    source,
                }
            }
        })?;

        let duration_ms = started_at.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

        if output.status.success() {
            return Ok(GitOutput::new(
                output.status.code(),
                stdout,
                stderr,
                duration_ms,
            ));
        }

        Err(GitExecError::NonZeroExit {
            code: output.status.code(),
            args: command.args.clone(),
            stdout,
            stderr,
        })
    }
}

/// Records commands without executing them so command-building behavior can be tested in isolation.
#[derive(Debug, Default, Clone)]
pub struct RecordingGitRunner;

impl GitRunner for RecordingGitRunner {
    /// Rejects execution because this runner exists to validate command assembly boundaries, not behavior.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        Ok(GitOutput::new(
            Some(0),
            String::new(),
            format!("recorded args: {:?}", command.args),
            0,
        ))
    }
}
