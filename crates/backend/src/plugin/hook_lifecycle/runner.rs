//! The execution boundary of a Hook lifecycle command: one contained package executable, spawned
//! without a shell, with bounded output and a hard time limit.
//!
//! The runner is the only place in the host that starts a package-provided program, so its
//! contract is deliberately narrow: it receives a program path and arguments the caller has
//! already resolved and validated, and it never consults a shell, a search path, or the package's
//! own text to expand them (Hook decision D4/D5). Everything else — which phase runs, whether the
//! user authorized it, what the result means for the install — stays with the orchestrator.

use ora_logging::ora_warn;
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, ProcessStdio, TokioProcessSpawner};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};

/// How long one lifecycle command may run before the host kills its process tree.
///
/// A Hook tool may legitimately do slow work — rewriting an Agent's configuration, migrating a
/// previous version's state — so this is generous enough to be a safety net rather than a
/// deadline. It exists because a package program is third-party code the host cannot inspect: an
/// operation that never returns would hold the plugin's operation lock forever.
pub(crate) const HOOK_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// How many bytes of one output stream the host captures.
///
/// A lifecycle command's value to the user is its exit status and the reason it failed, so the
/// capture only needs to keep enough of the stream to explain a failure; a tool that streams
/// progress indefinitely must not grow host memory with the run.
pub(crate) const MAX_HOOK_OUTPUT_BYTES: usize = 256 * 1024;

/// How much captured output a report carries to the caller.
///
/// The capture stays whole for the logs; what crosses the IPC boundary is bounded separately so
/// one verbose tool cannot push a quarter megabyte into a settings row.
pub(crate) const MAX_HOOK_REPORT_OUTPUT_BYTES: usize = 4 * 1024;

/// Describes one lifecycle command the host is about to run.
///
/// `program` is already the canonical path that containment validation resolved inside the
/// installed package; the runner spawns exactly it and never resolves a name of its own.
pub(crate) struct HookCommandSpec {
    pub program: PathBuf,
    /// Arguments declared by the package, passed verbatim and never through a shell.
    pub args: Vec<String>,
    /// The directory the child runs in, supplied by host policy rather than by the package.
    pub working_directory: PathBuf,
}

/// Holds one captured output stream together with its truncation state.
///
/// The default value is the empty capture, which is what an attempt that never started a process
/// — or one the host killed before it could be read — produced.
#[derive(Debug, Default)]
pub(crate) struct HookCommandOutput {
    pub text: String,
    pub truncated: bool,
}

/// Reports how a started command ended.
///
/// The platform's own status type stops at this boundary: the orchestrator only classifies an
/// attempt, so the process runner reduces a wait status to the code a report can carry, and the
/// code is absent for a process that was terminated rather than exited.
pub(crate) enum HookCommandFinished {
    /// The process exited on its own, with its platform exit code when it had one.
    Exited { code: Option<i32> },
    /// The host killed the process tree after [`HOOK_COMMAND_TIMEOUT`] elapsed.
    TimedOut,
}

/// The result of one attempt that reached the point of starting a process.
pub(crate) struct HookCommandExecution {
    pub finished: HookCommandFinished,
    pub stderr: HookCommandOutput,
}

/// Reports that the host could not start the command at all.
///
/// This is separated from a non-zero exit because the two are different things to tell the user:
/// a command that started and failed produced output explaining itself, while a command that
/// never started means the installed package no longer matches what validation accepted.
#[derive(Debug)]
pub(crate) struct HookCommandError {
    pub message: String,
}

/// Runs one Hook lifecycle command.
///
/// Implementations are the seam that keeps the orchestration testable: the production runner
/// starts a real child process, while tests substitute a runner that records the spec it was
/// given and returns a scripted result without executing anything.
pub(crate) trait HookCommandRunner: Send + Sync + 'static {
    /// Runs `spec` to completion under the host's own limits.
    fn run(
        &self,
        spec: &HookCommandSpec,
    ) -> impl Future<Output = Result<HookCommandExecution, HookCommandError>> + Send;
}

/// Dyn-compatible mirror of [`HookCommandRunner`], boxed so the slot can hold any runner type.
trait ErasedHookCommandRunner: Send + Sync {
    /// Boxed form of [`HookCommandRunner::run`].
    fn run_erased<'a>(
        &'a self,
        spec: &'a HookCommandSpec,
    ) -> Pin<Box<dyn Future<Output = Result<HookCommandExecution, HookCommandError>> + Send + 'a>>;
}

impl<Runner: HookCommandRunner> ErasedHookCommandRunner for Runner {
    /// Boxes the statically dispatched future so the slot can hold any runner type.
    fn run_erased<'a>(
        &'a self,
        spec: &'a HookCommandSpec,
    ) -> Pin<Box<dyn Future<Output = Result<HookCommandExecution, HookCommandError>> + Send + 'a>>
    {
        Box::pin(self.run(spec))
    }
}

/// Holds the runner the host executes through, replaceable so tests never spawn a real process.
pub(crate) struct HookCommandRunnerSlot {
    runner: RwLock<Arc<dyn ErasedHookCommandRunner>>,
}

impl Default for HookCommandRunnerSlot {
    /// Installs the production process runner, which is what every non-test host needs.
    fn default() -> Self {
        Self::new(ProcessHookCommandRunner)
    }
}

impl HookCommandRunnerSlot {
    /// Holds `runner` as the executor of every later lifecycle command.
    pub(crate) fn new(runner: impl HookCommandRunner) -> Self {
        Self {
            runner: RwLock::new(Arc::new(runner)),
        }
    }

    /// Replaces the runner; later commands use the new one.
    #[cfg(test)]
    pub(crate) fn install(&self, runner: impl HookCommandRunner) {
        *self.runner.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(runner);
    }

    /// Runs one command through the installed runner.
    pub(crate) async fn run(
        &self,
        spec: &HookCommandSpec,
    ) -> Result<HookCommandExecution, HookCommandError> {
        // Clone out of the lock so the run never holds the guard across an await.
        let runner = self
            .runner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        runner.run_erased(spec).await
    }
}

/// Starts the resolved package executable as a real child process.
///
/// The child inherits the host environment — the tool is expected to discover the Agent
/// installations and user configuration it manages from its own environment — but receives no
/// Hook setting value, no credential, and no argument the host invented: the package declares its
/// own arguments verbatim.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProcessHookCommandRunner;

impl HookCommandRunner for ProcessHookCommandRunner {
    /// Spawns the child with a closed stdin and piped output, kills its process tree on timeout,
    /// and captures a bounded tail of its standard error.
    async fn run(&self, spec: &HookCommandSpec) -> Result<HookCommandExecution, HookCommandError> {
        // No reaper registration: the reaper owns plugin runtime processes, whose lifetime is the
        // host's, while a lifecycle command is a short-lived tool the host waits for and kills by
        // process tree itself.
        let process_spec = ProcessSpec::new(spec.program.as_os_str())
            .args(&spec.args)
            .cwd(spec.working_directory.clone())
            .stdin(ProcessStdio::Null)
            .stdout(ProcessStdio::Piped)
            .stderr(ProcessStdio::Piped)
            .skip_reaper_registration();
        let mut process = TokioProcessSpawner::new()
            .spawn(process_spec)
            .map_err(|error| HookCommandError {
                message: format!("failed to start `{}`: {error}", spec.program.display()),
            })?;
        let stdout = take_pipe(process.take_stdout(), &spec.program, "stdout")?;
        let stderr = take_pipe(process.take_stderr(), &spec.program, "stderr")?;

        let completion = collect(&process, stdout, stderr);
        match tokio::time::timeout(HOOK_COMMAND_TIMEOUT, completion).await {
            Ok(result) => result,
            Err(_) => {
                // The kill is tree-wide, so a tool that spawned helpers of its own leaves nothing
                // running behind it. A refused kill still ends the attempt as a timeout: the
                // command did not finish, and the report says so either way.
                if let Err(error) = process.kill().await {
                    ora_warn!(
                        program = %spec.program.display(),
                        %error,
                        "failed to kill a Hook lifecycle command after the timeout"
                    );
                }
                Ok(HookCommandExecution {
                    finished: HookCommandFinished::TimedOut,
                    // Whatever the command wrote before the kill is lost with the dropped readers;
                    // the reason in the report names the limit instead of pretending to quote it.
                    stderr: HookCommandOutput {
                        text: String::new(),
                        truncated: false,
                    },
                })
            }
        }
    }
}

/// Takes one stdio pipe out of the spawned handle, reporting an unavailable pipe as a start error.
fn take_pipe<Pipe>(
    pipe: Option<Pipe>,
    program: &Path,
    name: &str,
) -> Result<Pipe, HookCommandError> {
    pipe.ok_or_else(|| HookCommandError {
        message: format!("`{}` did not expose a {name} pipe", program.display()),
    })
}

/// Reads both pipes concurrently while the process exits so neither pipe can deadlock the child.
///
/// Standard output is drained and discarded: nothing in a lifecycle command's success path is
/// interpreted, and only standard error is kept to explain a failure.
async fn collect<Process, Stdout, Stderr>(
    process: &Process,
    stdout: Stdout,
    stderr: Stderr,
) -> Result<HookCommandExecution, HookCommandError>
where
    Process: ManagedProcess,
    Stdout: AsyncRead + Unpin,
    Stderr: AsyncRead + Unpin,
{
    let (_, stderr, status) =
        tokio::join!(read_bounded(stdout), read_bounded(stderr), process.wait());
    let stderr = stderr.map_err(|error| HookCommandError {
        message: format!("failed to read the command's standard error: {error}"),
    })?;
    let status = status.map_err(|error| HookCommandError {
        message: format!("failed to wait for the command: {error}"),
    })?;
    Ok(HookCommandExecution {
        finished: HookCommandFinished::Exited {
            code: status.code(),
        },
        stderr: bounded_text(&stderr, MAX_HOOK_OUTPUT_BYTES),
    })
}

/// Reads one pipe with a sentinel byte so callers can distinguish exact-limit output from overflow.
async fn read_bounded<R>(reader: R) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    reader
        .take((MAX_HOOK_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .await?;
    Ok(output)
}

/// Renders one captured stream as trimmed text and reports whether the capture was cut short.
fn bounded_text(captured: &[u8], limit: usize) -> HookCommandOutput {
    let truncated = captured.len() > limit;
    let kept = &captured[..captured.len().min(limit)];
    HookCommandOutput {
        text: String::from_utf8_lossy(kept).trim().to_string(),
        truncated,
    }
}

/// Bounds one captured stream for a report, independently of the capture limit.
///
/// The cut is taken on a character boundary so a multi-byte sequence is never split into
/// replacement characters the tool never wrote.
pub(crate) fn report_output(output: &HookCommandOutput) -> HookCommandOutput {
    if output.text.len() <= MAX_HOOK_REPORT_OUTPUT_BYTES {
        return HookCommandOutput {
            text: output.text.clone(),
            truncated: output.truncated,
        };
    }
    let mut end = MAX_HOOK_REPORT_OUTPUT_BYTES;
    while end > 0 && !output.text.is_char_boundary(end) {
        end -= 1;
    }
    HookCommandOutput {
        text: output.text[..end].to_string(),
        truncated: true,
    }
}
