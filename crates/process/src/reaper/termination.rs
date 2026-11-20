//! One-shot final cleanup shared by the protocol loop and, on Unix, the termination-signal thread.
//!
//! Parent EOF, an explicit `SHUTDOWN` request and a termination signal can race. All of them end
//! in [`SharedTargets::finish`], so exactly one caller runs `kill_all` and no request registers a
//! tree after cleanup started. The mutex stays held for the whole `kill_all`; that only submits
//! `SIGKILL`s or one `TerminateJobObject` and never waits for a process to exit, so a competing
//! caller waits for a bounded time.
//!
//! Signals are never handled in a signal handler. On Unix the termination signals are blocked
//! before the reaper starts any other thread, so every thread inherits the mask, and one dedicated
//! thread receives them synchronously with `sigwait`. That thread runs ordinary Rust code: it
//! takes the same mutex as the protocol loop, which may meanwhile stay blocked in its stdin read.
//!
//! A signal whose disposition the parent set to `SIG_IGN` behaves differently by platform, since
//! POSIX leaves it unspecified whether such a signal is discarded at generation while blocked.
//! Linux keeps it pending, so `sigwait` still receives it and the reaper cleans up. macOS (XNU)
//! discards it, so the reaper keeps running as if it was never sent; a non-interactive shell's
//! background job, for example, inherits an ignored SIGINT, and `nohup` ignores SIGHUP. No tree
//! is stranded either way, because an ignored signal does not end the reaper.
//!
//! Windows has no termination signals. Anything that ends the reaper there, `TerminateProcess`
//! and console control events included, closes the only handle to its aggregate Job Object, whose
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` limit makes the kernel end every registered tree. Windows
//! therefore installs nothing here.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::tree::ReaperTargets;

/// Registered trees, shared between the protocol loop and the code that runs the final cleanup.
#[derive(Clone)]
pub(super) struct SharedTargets(Arc<Mutex<TargetsState>>);

/// Whether the reaper still accepts registrations or has already terminated its trees.
enum TargetsState {
    Serving(ReaperTargets),
    Finished,
}

/// What a call to [`SharedTargets::finish`] did.
pub(super) enum Cleanup<'a> {
    /// This caller ran the final `kill_all`. Every other caller waits until `hold` is dropped, so
    /// a process that exits while holding it never serves another request.
    Performed {
        result: io::Result<()>,
        hold: CleanupHold<'a>,
    },
    /// Another caller already ran the final cleanup.
    AlreadyFinished,
}

/// Keeps the targets locked after the final cleanup.
pub(super) struct CleanupHold<'a> {
    _guard: MutexGuard<'a, TargetsState>,
}

impl SharedTargets {
    pub(super) fn new(targets: ReaperTargets) -> Self {
        Self(Arc::new(Mutex::new(TargetsState::Serving(targets))))
    }

    /// Runs one registration request, failing once the final cleanup has started.
    pub(super) fn serve(
        &self,
        request: impl FnOnce(&mut ReaperTargets) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match &mut *state {
            TargetsState::Serving(targets) => request(targets),
            TargetsState::Finished => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "process reaper already terminated its registered trees",
            )),
        }
    }

    /// Terminates every registered tree unless another caller already did.
    pub(super) fn finish(&self) -> Cleanup<'_> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match std::mem::replace(&mut *state, TargetsState::Finished) {
            TargetsState::Serving(mut targets) => Cleanup::Performed {
                result: targets.kill_all(),
                hold: CleanupHold { _guard: state },
            },
            TargetsState::Finished => Cleanup::AlreadyFinished,
        }
    }
}

impl Cleanup<'_> {
    /// Reports the cleanup outcome to a caller that keeps running afterwards; a cleanup another
    /// caller already ran counts as done.
    pub(super) fn into_result(self) -> io::Result<()> {
        match self {
            Self::Performed { result, hold } => {
                drop(hold);
                result
            }
            Self::AlreadyFinished => Ok(()),
        }
    }
}

/// What the signal thread does after a termination signal.
#[cfg(unix)]
enum SignalResponse<'a> {
    /// Exit the process with `code`. `hold` must stay alive until the process is gone: releasing
    /// it would let the protocol loop serve a request against finished targets, answer it with a
    /// failure, and race this exit with its own unsuccessful one.
    Exit { code: i32, hold: CleanupHold<'a> },
    /// The protocol loop already ran the cleanup and exits once it has sent its last response;
    /// exiting here could drop that `SHUTDOWN` acknowledgement.
    LeaveExitToProtocolLoop,
}

/// Decides how the signal thread ends after the final cleanup the signal requested.
#[cfg(unix)]
fn respond_to_termination_signal(cleanup: Cleanup<'_>, signal: i32) -> SignalResponse<'_> {
    match cleanup {
        Cleanup::Performed {
            result: Ok(()),
            hold,
        } => SignalResponse::Exit { code: 0, hold },
        Cleanup::Performed {
            result: Err(error),
            hold,
        } => {
            eprintln!("ora-reaper failed to clean up after signal {signal}: {error}");
            SignalResponse::Exit { code: 1, hold }
        }
        Cleanup::AlreadyFinished => SignalResponse::LeaveExitToProtocolLoop,
    }
}

/// Makes SIGTERM, SIGINT and SIGHUP run the final cleanup and then exit the reaper.
///
/// Each one would otherwise end the reaper with its default action and strand the registered
/// trees. The reaper sits outside Ora's process group, so these arrive by PID: SIGTERM from a
/// cleanup owner or a service manager, SIGINT from an operator, SIGHUP when the reaper's session
/// or terminal goes away, which, like parent EOF, means its supervisor is gone. SIGQUIT keeps its
/// core-dump default for diagnostics, and SIGKILL cannot be caught. On macOS a signal inherited
/// as ignored stays ignored; see the module documentation.
///
/// The caller must not have started any other thread: a thread that does not inherit the blocked
/// mask could receive a signal with its default action. Returns an error, leaving the signals
/// blocked, when the waiting thread cannot start; the reaper then fails before announcing
/// readiness instead of running unable to clean up on a signal.
#[cfg(unix)]
pub(super) fn clean_up_on_termination_signals(targets: &SharedTargets) -> io::Result<()> {
    // SAFETY: `sigemptyset` initializes the set before `sigaddset` reads it, and both only write
    // the local value with valid signal numbers.
    let signals = unsafe {
        let mut signals = std::mem::zeroed::<libc::sigset_t>();
        libc::sigemptyset(&mut signals);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut signals, signal);
        }
        signals
    };
    // SAFETY: the set is initialized above, and the old mask is not requested.
    let result = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &signals, std::ptr::null_mut()) };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    // A weak reference keeps the protocol loop the owner of the targets: if it unwinds, dropping
    // them still runs the `ReaperTargets` fail-safe cleanup.
    let targets = Arc::downgrade(&targets.0);
    std::thread::Builder::new()
        .name("ora-reaper-signals".to_owned())
        .spawn(move || wait_for_termination_signals(&signals, &targets))?;
    Ok(())
}

/// Receives the blocked termination signals and runs the final cleanup on the first one.
///
/// Repeated signals are ignored rather than abort a cleanup halfway: they stay blocked, so they
/// are only coalesced while this thread runs `kill_all`, and the process exits right after it.
/// When the protocol loop already ran the cleanup, this thread leaves the exit to that loop, which
/// is only sending its last acknowledgement.
#[cfg(unix)]
fn wait_for_termination_signals(
    signals: &libc::sigset_t,
    targets: &std::sync::Weak<Mutex<TargetsState>>,
) {
    loop {
        let mut signal = 0;
        // SAFETY: the set is initialized, and `signal` is a valid output location.
        let result = unsafe { libc::sigwait(signals, &mut signal) };
        if result != 0 {
            // `sigwait` only rejects an invalid set. Unblocking hands the signals back to their
            // default action, the behavior without this thread, instead of ignoring them forever.
            eprintln!(
                "ora-reaper cannot wait for termination signals: {}",
                io::Error::from_raw_os_error(result)
            );
            // SAFETY: the set is initialized, and the old mask is not requested.
            unsafe { libc::pthread_sigmask(libc::SIG_UNBLOCK, signals, std::ptr::null_mut()) };
            return;
        }
        // The protocol loop dropped the targets: it already returned and the process is exiting.
        let Some(targets) = targets.upgrade().map(SharedTargets) else {
            continue;
        };
        ora_logging::ora_trace!(signal, "process reaper received a termination signal");
        match respond_to_termination_signal(targets.finish(), signal) {
            // `exit` never returns, so `_hold` keeps the targets locked until the process is gone.
            // No test can observe a release here: it would race this very exit.
            SignalResponse::Exit { code, hold: _hold } => std::process::exit(code),
            SignalResponse::LeaveExitToProtocolLoop => {}
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Child, Command};
    use std::sync::mpsc;
    use std::time::Duration;

    use pretty_assertions::assert_eq;

    use super::{Cleanup, SharedTargets, SignalResponse, respond_to_termination_signal};
    use crate::tree::ReaperTargets;

    /// A cleanup outcome reduced to what the assertions compare.
    #[derive(Debug, PartialEq)]
    enum Outcome {
        Performed,
        AlreadyFinished,
    }

    /// A signal response reduced to what the assertions compare.
    #[derive(Debug, PartialEq)]
    enum Response {
        /// The exit code, and whether the targets were still locked when the response was given.
        Exit {
            code: i32,
            targets_locked: bool,
        },
        LeaveExitToProtocolLoop,
    }

    /// A signal that runs the cleanup kills the registered group and exits successfully, still
    /// holding the targets so the protocol loop cannot serve another request before the exit.
    #[test]
    fn a_signal_that_cleans_up_exits_successfully_holding_the_targets() {
        let mut target = spawn_group_leader();
        let targets = registered_targets(&target);

        let response = response(
            &targets,
            respond_to_termination_signal(targets.finish(), libc::SIGTERM),
        );
        let target_signal = target
            .wait()
            .unwrap_or_else(|error| panic!("wait for target: {error}"))
            .signal();

        assert_eq!(
            (response, target_signal),
            (
                Response::Exit {
                    code: 0,
                    targets_locked: true
                },
                Some(libc::SIGKILL)
            )
        );
    }

    /// A signal whose cleanup fails exits unsuccessfully, still holding the targets.
    #[test]
    fn a_signal_whose_cleanup_fails_exits_with_failure() {
        let targets = empty_targets();
        let Cleanup::Performed { result: _, hold } = targets.finish() else {
            panic!("fresh targets must run the final cleanup");
        };
        let failed = Cleanup::Performed {
            result: Err(io::Error::other("injected kill failure")),
            hold,
        };

        let response = response(
            &targets,
            respond_to_termination_signal(failed, libc::SIGTERM),
        );

        assert_eq!(
            response,
            Response::Exit {
                code: 1,
                targets_locked: true
            }
        );
    }

    /// A signal after the protocol loop's own cleanup leaves the exit to that loop, so its last
    /// `SHUTDOWN` acknowledgement is not lost.
    #[test]
    fn a_signal_after_protocol_cleanup_leaves_the_exit_to_the_protocol_loop() {
        let targets = empty_targets();
        targets
            .finish()
            .into_result()
            .unwrap_or_else(|error| panic!("protocol cleanup: {error}"));

        let response = response(
            &targets,
            respond_to_termination_signal(targets.finish(), libc::SIGHUP),
        );

        assert_eq!(response, Response::LeaveExitToProtocolLoop);
    }

    /// The first caller terminates the registered group; later callers, whichever path they come
    /// from, neither run `kill_all` again nor register a new tree.
    #[test]
    fn final_cleanup_runs_once() {
        let mut target = spawn_group_leader();
        let targets = registered_targets(&target);

        let first = outcome(targets.finish());
        let target_signal = target
            .wait()
            .unwrap_or_else(|error| panic!("wait for target: {error}"))
            .signal();
        let second = outcome(targets.finish());
        let register = targets
            .serve(|targets| targets.register(std::process::id()))
            .map_err(|error| error.kind());

        assert_eq!(
            (first, target_signal, second, register),
            (
                Outcome::Performed,
                Some(libc::SIGKILL),
                Outcome::AlreadyFinished,
                Err(io::ErrorKind::BrokenPipe)
            )
        );
    }

    /// A caller that arrives while another one cleans up waits for it instead of starting a second
    /// `kill_all` or returning early, then finds the cleanup done.
    #[test]
    fn competing_cleanup_waits_for_the_running_one() {
        let mut target = spawn_group_leader();
        let targets = registered_targets(&target);
        let running = targets.finish();
        let (sender, receiver) = mpsc::channel();
        let competitor = targets.clone();
        let handle = std::thread::spawn(move || {
            let _ = sender.send(outcome(competitor.finish()));
        });

        let while_running = receiver.recv_timeout(Duration::from_millis(100)).ok();
        let first = outcome(running);
        let after_release = receiver.recv_timeout(Duration::from_secs(5)).ok();
        handle
            .join()
            .unwrap_or_else(|_| panic!("competing cleanup thread panicked"));
        let target_signal = target
            .wait()
            .unwrap_or_else(|error| panic!("wait for target: {error}"))
            .signal();

        assert_eq!(
            (while_running, first, after_release, target_signal),
            (
                None,
                Outcome::Performed,
                Some(Outcome::AlreadyFinished),
                Some(libc::SIGKILL)
            )
        );
    }

    /// Starts a process that leads its own group and stays alive until it is killed.
    fn spawn_group_leader() -> Child {
        Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .process_group(0)
            .spawn()
            .unwrap_or_else(|error| panic!("spawn target: {error}"))
    }

    /// Registers one live group with fresh shared targets.
    fn registered_targets(target: &Child) -> SharedTargets {
        let targets = SharedTargets::new(
            ReaperTargets::new().unwrap_or_else(|error| panic!("create targets: {error}")),
        );
        targets
            .serve(|targets| targets.register(target.id()))
            .unwrap_or_else(|error| panic!("register target: {error}"));
        targets
    }

    /// Creates shared targets with nothing registered.
    fn empty_targets() -> SharedTargets {
        SharedTargets::new(
            ReaperTargets::new().unwrap_or_else(|error| panic!("create targets: {error}")),
        )
    }

    /// Records a signal response, probing the lock before the response releases it.
    fn response(targets: &SharedTargets, response: SignalResponse<'_>) -> Response {
        // `try_lock` on the same thread fails without blocking while any guard is alive.
        let targets_locked = targets.0.try_lock().is_err();
        match response {
            SignalResponse::Exit { code, hold: _ } => Response::Exit {
                code,
                targets_locked,
            },
            SignalResponse::LeaveExitToProtocolLoop => Response::LeaveExitToProtocolLoop,
        }
    }

    /// Drops the hold, releasing waiting callers, and checks that a cleanup succeeded.
    fn outcome(cleanup: Cleanup<'_>) -> Outcome {
        match cleanup {
            Cleanup::Performed { result, hold: _ } => {
                result.unwrap_or_else(|error| panic!("final cleanup: {error}"));
                Outcome::Performed
            }
            Cleanup::AlreadyFinished => Outcome::AlreadyFinished,
        }
    }
}
