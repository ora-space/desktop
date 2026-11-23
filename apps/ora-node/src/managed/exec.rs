//! Runs one Git command as a direct child of the Node and settles how it ended.
//!
//! Git leads its own process group and is registered with the Node's reaper, so a deadline, a
//! Node stop or the Node's death ends the whole group. A command with side effects also inherits
//! its attempt lock; its result is only reported once that lock could be taken again, because
//! only then is nothing of its process tree left to write.
use super::{
    AttemptSettlement, ProcessConfig, RunAs, Shutdown,
    attempt_lock::{AttemptLocks, StopEvidence, kill_group},
};
use gitlancer::{GitCommand, GitOutput};
use ora_node_db::AttemptId;
use ora_process::{
    ManagedProcess, ProcessEnvironment, ProcessSpawner, ProcessSpec, ProcessStdio,
    TokioProcessSpawner,
};
use std::{
    ffi::OsString,
    io,
    os::unix::process::ExitStatusExt,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, sync::Notify, task::JoinHandle};

/// Git must not detach maintenance after a fetch: it would inherit the attempt lock and hold the
/// stop evidence back until work nobody owns has finished.
const NO_BACKGROUND_MAINTENANCE: [&str; 4] = ["-c", "gc.auto=0", "-c", "maintenance.auto=false"];

/// How much of the command's output the caller wants back.
#[derive(Clone, Copy)]
pub(super) enum Capture {
    /// Both streams go to the null device; only the ending matters.
    Discard,
    /// Both streams are read into memory; one byte beyond a limit fails the command.
    Bounded { stdout: usize, stderr: usize },
}

/// Whether a Node stop cuts the command short once the stop grace has passed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StopGrace {
    Honored,
    /// A restore command is bounded by its own absolute cutoff instead.
    Ignored,
}

/// One command to run, with everything that decides how long it may take.
pub(super) struct Invocation<'a> {
    pub(super) command: &'a GitCommand,
    pub(super) environment: Vec<(OsString, OsString)>,
    pub(super) identity: RunAs,
    pub(super) capture: Capture,
    pub(super) deadline: Instant,
    pub(super) stop_grace: StopGrace,
    /// Runs right before the command is cut short, so a caller can anchor its own stop clock
    /// ahead of the kill.
    pub(super) before_cut: Option<&'a dyn Fn()>,
    /// The attempt whose lock the command inherits; `None` for read-only commands, whose
    /// leftovers cannot write and need no stop evidence.
    pub(super) lock: Option<&'a AttemptId>,
}

/// How the command ended and what it printed.
pub(super) struct Finished {
    pub(super) settlement: AttemptSettlement,
    /// The output of a command that ran to its own end. A deadline, an exceeded output limit and
    /// missing stop evidence are errors even when Git happened to exit by itself.
    pub(super) output: io::Result<GitOutput>,
}

/// Runs the command to its end and waits for its stop evidence before returning.
pub(super) async fn run(
    config: &ProcessConfig,
    locks: &AttemptLocks,
    shutdown: &Shutdown,
    invocation: Invocation<'_>,
) -> Finished {
    let started = Instant::now();
    let cleanup = Duration::from_millis(config.cleanup_timeout_ms);
    let launched = launch(config, locks, shutdown, &invocation);
    let mut process = match launched {
        Ok(process) => process,
        Err(error) => {
            // Nothing was started, or the spawn failed before Git ran: the lock tells which.
            let evidence = match invocation.lock {
                Some(attempt) => locks.await_stop(attempt, cleanup).await,
                None => StopEvidence::Stopped,
            };
            return Finished {
                settlement: match evidence {
                    StopEvidence::Stopped => AttemptSettlement::Stopped,
                    StopEvidence::Running | StopEvidence::Unknown => AttemptSettlement::Unverified,
                },
                output: Err(error),
            };
        }
    };
    let overflow = Arc::new(Notify::new());
    let (stdout_limit, stderr_limit) = match invocation.capture {
        Capture::Discard => (0, 0),
        Capture::Bounded { stdout, stderr } => (stdout, stderr),
    };
    let stdout = process
        .take_stdout()
        .map(|pipe| tokio::spawn(read_bounded(pipe, stdout_limit, overflow.clone())));
    let stderr = process
        .take_stderr()
        .map(|pipe| tokio::spawn(read_bounded(pipe, stderr_limit, overflow.clone())));

    let grace = Duration::from_millis(config.shutdown_grace_ms);
    let mut tick = tokio::time::interval(Duration::from_millis(/*millis*/ 25));
    let mut cut: Option<io::Error> = None;
    let status = loop {
        tokio::select! {
            status = process.wait() => break status,
            () = tokio::time::sleep_until(invocation.deadline.into()), if cut.is_none() => {
                cut = Some(io::Error::new(io::ErrorKind::TimedOut, "Git completion deadline expired"));
            }
            _ = tick.tick(), if cut.is_none() && invocation.stop_grace == StopGrace::Honored => {
                if shutdown.expired(grace) {
                    cut = Some(io::Error::new(io::ErrorKind::TimedOut, "Git completion deadline expired"));
                }
            }
            () = overflow.notified(), if cut.is_none() => {
                cut = Some(io::Error::other("Git output is truncated or unavailable"));
            }
        }
        if cut.is_some() {
            if let Some(before_cut) = invocation.before_cut {
                before_cut();
            }
            // Ends the whole group; the loop keeps waiting for the direct child's own exit.
            let _ = process.kill().await;
        }
    };
    // Whatever Git left in its group has no owner any more, whether Git exited or was killed.
    if let Some(pid) = process.id() {
        kill_group(pid);
    }
    // A descendant that left the group may keep a pipe open; its readers are abandoned rather
    // than awaited beyond the cleanup budget.
    let readers = [&stdout, &stderr].map(|reader| reader.as_ref().map(JoinHandle::abort_handle));
    let collected = tokio::time::timeout(cleanup, async {
        Ok::<_, io::Error>((collect(stdout).await?, collect(stderr).await?))
    })
    .await
    .unwrap_or_else(|_| {
        readers
            .iter()
            .flatten()
            .for_each(tokio::task::AbortHandle::abort);
        Err(io::Error::other("Git output EOF is unverified"))
    });
    let evidence = match invocation.lock {
        Some(attempt) => locks.await_stop(attempt, cleanup).await,
        None => StopEvidence::Stopped,
    };
    let settlement = match (evidence, &status) {
        (StopEvidence::Stopped, Ok(status)) => match (status.code(), status.signal()) {
            (Some(code), _) => AttemptSettlement::Exited(code),
            (None, Some(signal)) => AttemptSettlement::Terminated(signal),
            (None, None) => AttemptSettlement::Stopped,
        },
        (StopEvidence::Stopped, Err(_)) => AttemptSettlement::Stopped,
        (StopEvidence::Running | StopEvidence::Unknown, _) => AttemptSettlement::Unverified,
    };
    let output = match (settlement, cut, collected) {
        (AttemptSettlement::Unverified, _, _) => {
            Err(io::Error::other("Git stop evidence is unverified"))
        }
        (_, Some(error), _) | (_, None, Err(error)) => Err(error),
        (settlement, None, Ok((stdout, stderr))) => Ok(GitOutput::new(
            match settlement {
                AttemptSettlement::Exited(code) => Some(code),
                AttemptSettlement::Terminated(_)
                | AttemptSettlement::Stopped
                | AttemptSettlement::Unverified => None,
            },
            stdout,
            stderr,
            started.elapsed().as_millis() as u64,
        )),
    };
    Finished { settlement, output }
}

/// Takes the attempt lock and starts Git with it, leaving the lock held by Git's tree alone.
fn launch(
    config: &ProcessConfig,
    locks: &AttemptLocks,
    shutdown: &Shutdown,
    invocation: &Invocation<'_>,
) -> io::Result<ora_process::TokioManagedProcess> {
    if ora_process::reaper_has_exited() {
        // Without the reaper nothing ends Git if this Node dies next, so the Node stops instead
        // of running on with a guarantee it no longer has.
        shutdown.fail("the process reaper exited");
        return Err(io::Error::other("the process reaper exited"));
    }
    let (stdout, stderr) = match invocation.capture {
        Capture::Discard => (ProcessStdio::Null, ProcessStdio::Null),
        Capture::Bounded { .. } => (ProcessStdio::Piped, ProcessStdio::Piped),
    };
    let mut spec = ProcessSpec::new(config.git_program.as_os_str())
        .args(NO_BACKGROUND_MAINTENANCE)
        .args(&invocation.command.args)
        .cwd(&invocation.command.cwd)
        .stdin(ProcessStdio::Null)
        .stdout(stdout)
        .stderr(stderr);
    for (key, value) in &invocation.environment {
        spec = spec.env(key, value);
    }
    let descriptor = invocation
        .lock
        .map(|attempt| locks.hold(attempt))
        .transpose()?;
    if let Some(descriptor) = &descriptor {
        spec = spec.inherit_descriptor(descriptor.clone());
    }
    let process = TokioProcessSpawner::running_as(config.identity(invocation.identity)?)
        .with_environment(ProcessEnvironment::Empty)
        .spawn(spec)?;
    // The spec released its share when the spawn returned; dropping this one leaves the lock
    // with Git's process tree only, so it frees itself when that tree is gone.
    drop(descriptor);
    if let (Some(attempt), Some(pid)) = (invocation.lock, process.id()) {
        locks.note_child(attempt, pid);
    }
    Ok(process)
}

/// Reads one stream to its end, refusing more than `limit` bytes.
///
/// Truncated output must never reach a Git fact parser, so the overflow is reported to the
/// waiter, which ends the command instead of letting it block on a full pipe.
async fn read_bounded(
    pipe: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
    overflow: Arc<Notify>,
) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > limit {
        overflow.notify_one();
        return Err(io::Error::other("Git output is truncated or unavailable"));
    }
    Ok(bytes)
}

/// Joins one reader; a stream that was never piped is empty.
async fn collect(reader: Option<JoinHandle<io::Result<Vec<u8>>>>) -> io::Result<String> {
    match reader {
        Some(reader) => {
            let bytes = reader.await.map_err(io::Error::other)??;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        None => Ok(String::new()),
    }
}
