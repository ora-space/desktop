use std::io::{ErrorKind, Read};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use std::time::Instant;

use crate::error::GitExecError;
use crate::exec::command::GitCommand;
use crate::exec::output::GitOutput;
use crate::logging;

/// Executes one prepared Git command and returns normalized process output.
pub trait GitRunner {
    /// Runs a fully assembled Git command so use-case layers stay independent from process details.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError>;

    /// Runs a command while bounding both captured streams before they enter application memory.
    fn run_bounded(
        &self,
        command: &GitCommand,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Result<GitOutput, GitExecError> {
        let output = self.run(command)?;
        if output.stdout.len() > max_stdout_bytes {
            return Err(GitExecError::OutputTooLarge {
                stream: "stdout",
                limit: max_stdout_bytes,
            });
        }
        if output.stderr.len() > max_stderr_bytes {
            return Err(GitExecError::OutputTooLarge {
                stream: "stderr",
                limit: max_stderr_bytes,
            });
        }
        Ok(output)
    }
}

/// Executes Git commands through the system `git` binary.
#[derive(Debug, Default, Clone, Copy)]
pub struct CliGitRunner;

impl GitRunner for CliGitRunner {
    /// Spawns the Git CLI with stable automation defaults so upper layers can trust execution semantics.
    fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
        run_cli(command, /*limits*/ None)
    }

    /// Captures Git output concurrently so neither pipe can deadlock or grow without a bound.
    fn run_bounded(
        &self,
        command: &GitCommand,
        max_stdout_bytes: usize,
        max_stderr_bytes: usize,
    ) -> Result<GitOutput, GitExecError> {
        run_cli(command, Some((max_stdout_bytes, max_stderr_bytes)))
    }
}

/// Executes one CLI command with optional per-stream capture limits.
fn run_cli(
    command: &GitCommand,
    limits: Option<(usize, usize)>,
) -> Result<GitOutput, GitExecError> {
    let logger = logging::get();
    let started_at = Instant::now();

    let full_command = format!("git {}", command.args.join(" "));
    if let Some(ref l) = logger {
        l.log_command(&command.cwd, &full_command);
    }

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
    process.envs(&command.env.variables);

    process.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(source) => {
            if let Some(ref l) = logger {
                l.log_result(0, false, None);
            }
            return Err(if source.kind() == ErrorKind::NotFound {
                GitExecError::GitNotFound
            } else {
                GitExecError::SpawnFailed {
                    args: command.args.clone(),
                    source,
                }
            });
        }
    };

    let stdout = child.stdout.take().expect("piped stdout must be available");
    let stderr = child.stderr.take().expect("piped stderr must be available");
    let (stdout_limit, stderr_limit) = limits.unwrap_or((usize::MAX, usize::MAX));
    let output_exceeded = Arc::new(AtomicBool::new(false));
    let stdout_exceeded = Arc::clone(&output_exceeded);
    let stderr_exceeded = Arc::clone(&output_exceeded);
    let stdout_reader =
        std::thread::spawn(move || read_bounded(stdout, stdout_limit, stdout_exceeded));
    let stderr_reader =
        std::thread::spawn(move || read_bounded(stderr, stderr_limit, stderr_exceeded));
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|source| GitExecError::OutputReadFailed {
                stream: "process status",
                source,
            })?
        {
            break status;
        }
        if output_exceeded.load(Ordering::Acquire) {
            // Once either stream exceeds its budget, killing Git bounds CPU and I/O as well as memory.
            // Git can exit between `try_wait` and `kill`; waiting still reaps it and lets the
            // bounded readers return the more useful `OutputTooLarge` error in that race.
            let _termination_result = child.kill();
            break child
                .wait()
                .map_err(|source| GitExecError::OutputReadFailed {
                    stream: "process status",
                    source,
                })?;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let stdout = join_bounded_reader(stdout_reader, "stdout", stdout_limit)?;
    let stderr = join_bounded_reader(stderr_reader, "stderr", stderr_limit)?;
    let duration_ms = started_at.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();

    if status.success() {
        if let Some(ref l) = logger {
            l.log_result(duration_ms, true, status.code());
        }
        return Ok(GitOutput::new(status.code(), stdout, stderr, duration_ms));
    }

    let code = status.code();
    if let Some(ref l) = logger {
        l.log_result(duration_ms, false, code);
    }
    Err(GitExecError::NonZeroExit {
        code,
        args: command.args.clone(),
        stdout,
        stderr,
    })
}

/// Reads one pipe without retaining bytes beyond its limit while continuing to drain the child.
fn read_bounded(
    mut reader: impl Read,
    limit: usize,
    output_exceeded: Arc<AtomicBool>,
) -> Result<(Vec<u8>, bool), std::io::Error> {
    let mut captured = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(captured.len());
        let retained = remaining.min(read);
        captured.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
        if truncated {
            output_exceeded.store(true, Ordering::Release);
        }
    }
    Ok((captured, truncated))
}

/// Maps pipe-reader failures and truncation into the stable Git execution error model.
fn join_bounded_reader(
    reader: std::thread::JoinHandle<Result<(Vec<u8>, bool), std::io::Error>>,
    stream: &'static str,
    limit: usize,
) -> Result<Vec<u8>, GitExecError> {
    let (bytes, truncated) = reader
        .join()
        .map_err(|_| GitExecError::OutputReadFailed {
            stream,
            source: std::io::Error::other("git output reader panicked"),
        })?
        .map_err(|source| GitExecError::OutputReadFailed { stream, source })?;
    if truncated {
        return Err(GitExecError::OutputTooLarge { stream, limit });
    }
    Ok(bytes)
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

#[cfg(test)]
mod tests {
    use super::read_bounded;
    use pretty_assertions::assert_eq;
    use std::sync::{Arc, atomic::AtomicBool};

    /// Verifies bounded readers drain input while retaining no bytes beyond the budget.
    #[test]
    fn bounds_captured_output() {
        assert_eq!(
            read_bounded("abcdef".as_bytes(), 3, Arc::new(AtomicBool::new(false))).unwrap(),
            (b"abc".to_vec(), true)
        );
    }
}
