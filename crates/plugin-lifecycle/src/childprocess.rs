//! Serves `ora/childprocess/*`: lets a plugin ask the host to spawn, write to, and kill a
//! subprocess, with its stdout, stderr, and exit pushed back as host-originated notifications.
//!
//! The host owns the OS process instead of the plugin's own sandboxed runtime spawning it
//! directly. It is created and torn down through `ora-process`'s tree-wide termination (a Windows
//! Job Object or a Unix process group), which is the same guarantee every other Ora-managed child
//! process already relies on. Every process tracked for one plugin generation is killed, best
//! effort, the moment that generation's [`PluginRuntime`](ora_plugin_runtime::PluginRuntime) stops
//! for any reason; see [`PluginProcessHost::kill_all`] and its wiring in
//! `runtime::DenoPluginRuntimeLauncher::launch`.
//!
//! A spawn request names its executable in one of two ways, and which one it uses decides who
//! resolves the path. `command` is handed to the operating system unchanged, for a PATH lookup or
//! a host-absolute path the plugin already knows. `packageCommand` is a package-relative path the
//! host joins onto this plugin's own install root, which is how a plugin runs an executable it
//! ships: the plugin is never told a host path (see `crate::runtime`, which injects no environment
//! and only sets the package root as the working directory), and it cannot reliably compute one —
//! a relative program combined with a `cwd` resolves against different directories per platform,
//! and the child's `cwd` must be the workspace rather than the package anyway.
//!
//! A `packageCommand` the package does not carry is reported as its own `package_command_missing`
//! kind, distinct from the `invalid_package_command` a present-but-unrunnable one gets. One plugin
//! source is built into both a package that bundles its CLI and one that leaves the user's own
//! install to be found on PATH, and it cannot know at build time which package it ended up in;
//! that distinction is the answer, and it lets a plugin fall back to `command` for the first case
//! while treating the second as the deterministic package fault it is.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ora_logging::ora_warn;
pub use ora_plugin_protocol::{
    CHILDPROCESS_CLOSE_STDIN_METHOD, CHILDPROCESS_KILL_METHOD, CHILDPROCESS_SPAWN_METHOD,
    CHILDPROCESS_WRITE_METHOD,
};
use ora_plugin_protocol::{
    CHILDPROCESS_EXIT_METHOD, CHILDPROCESS_STDERR_METHOD, CHILDPROCESS_STDOUT_METHOD,
    ChildProcessErrorKind, ChildProcessExit, ChildProcessIdParams, ChildProcessOutput,
    ChildProcessSpawnParams, ChildProcessSpawnResult, ChildProcessWriteParams,
    MAX_STORAGE_FILE_BYTES,
};
use ora_plugin_runtime::{
    HostRequestError, HostRequestHandler, PluginRuntime as ProcessPluginRuntime,
};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, ProcessStdio};
use ora_utils::path::{CanonicalPathRoot, PathContainmentError, PortableRelativePath};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

/// Chunk size used when pumping a spawned process's stdout or stderr into notifications.
const READ_CHUNK_BYTES: usize = 32 * 1024;

/// Upper bound on one `write` request's decoded payload, mirroring
/// [`ora_plugin_protocol::MAX_STORAGE_FILE_BYTES`] so a plugin cannot force unbounded host memory
/// growth by streaming an oversized chunk to a spawned process's stdin.
pub(crate) const MAX_WRITE_BYTES: usize = MAX_STORAGE_FILE_BYTES as usize;

/// Longest base64 string that can decode to `MAX_WRITE_BYTES`, checked before `BASE64.decode`
/// allocates so an oversized payload is rejected without ever being decoded.
const MAX_WRITE_BASE64_LEN: usize = MAX_WRITE_BYTES.div_ceil(3) * 4;

/// Reserves the `ORA_MCP_*` environment namespace so a plugin cannot smuggle
/// host-owned MCP secrets through a child-process spawn request.
const MCP_ENVIRONMENT_PREFIX: &str = "ORA_MCP_";

/// Supplies narrowly scoped environment variables for one host-managed Agent subprocess.
///
/// Implementations must derive values from host-owned configuration and must never include a
/// secret in an error. The host calls the provider only after binding the request to the calling
/// plugin and its requested workspace directory.
pub trait ChildProcessEnvironmentProvider: Clone + Send + Sync + 'static {
    /// Returns the variables authorized for this Agent plugin in this workspace.
    fn environment(
        &self,
        plugin_id: &str,
        workspace_root: &Path,
    ) -> Result<BTreeMap<String, String>, String>;
}

/// Leaves child-process environments unchanged when no host policy is configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoChildProcessEnvironment;

impl ChildProcessEnvironmentProvider for NoChildProcessEnvironment {
    fn environment(
        &self,
        _plugin_id: &str,
        _workspace_root: &Path,
    ) -> Result<BTreeMap<String, String>, String> {
        Ok(BTreeMap::new())
    }
}

/// One failed child-process call before it is rendered as a JSON-RPC error.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChildProcessError {
    kind: ChildProcessErrorKind,
    message: String,
}

impl ChildProcessError {
    fn new(kind: ChildProcessErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// Classifies a spawn failure, distinguishing a missing executable from any other I/O fault.
    fn from_spawn_io(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            Self::new(ChildProcessErrorKind::ProgramNotFound, error.to_string())
        } else {
            Self::new(ChildProcessErrorKind::Io, error.to_string())
        }
    }
}

impl From<ChildProcessError> for HostRequestError {
    fn from(error: ChildProcessError) -> Self {
        HostRequestError::new(error.kind.code(), error.message)
            .with_data(json!({ "kind": error.kind.as_str() }))
    }
}

/// One command the plugin asked the host to send to a spawned process's stdin.
enum StdinCommand {
    Write(Vec<u8>),
    Close,
}

/// One process this handler is tracking, keyed by its plugin-local `processId`.
struct Tracked<P> {
    process: Arc<P>,
    stdin_tx: mpsc::Sender<StdinCommand>,
    /// Joined by `watch_exit` before it pushes the exit notification: `wait()` and these pump
    /// tasks race independently against the same pipes, so without joining them first the exit
    /// notification could reach the plugin before the last stdout/stderr chunk does.
    stdout_done: JoinHandle<()>,
    stderr_done: JoinHandle<()>,
}

struct Inner<S: ProcessSpawner, E: ChildProcessEnvironmentProvider> {
    plugin_id: String,
    /// Install root of the package this handler serves, and the boundary every `packageCommand`
    /// must resolve inside. Fixed by the launch, so a request can never widen it.
    package_root: PathBuf,
    spawner: S,
    environment_provider: E,
    next_id: AtomicU64,
    tracked: StdMutex<HashMap<String, Tracked<S::Process>>>,
    /// Filled in once by [`PluginProcessHost::attach_runtime`] after the plugin connection this
    /// handler serves becomes ready; see the module docs for why this cannot be known upfront.
    runtime: watch::Sender<Option<ProcessPluginRuntime>>,
    /// Kept alive only so `runtime.send` above never observes zero receivers: `watch::Sender::send`
    /// fails (and drops its value) once every receiver is gone, and every other receiver used here
    /// is a short-lived `subscribe()` inside `push`. Never read directly.
    _runtime_rx: watch::Receiver<Option<ProcessPluginRuntime>>,
}

/// Serves `ora/childprocess/*` for one plugin process, spawning through `S`.
///
/// Generic over [`ProcessSpawner`] for the same reason `DenoPluginRuntimeLauncher` is: production
/// always uses [`ora_process::TokioProcessSpawner`], while tests inject a fake that never starts a
/// real OS process.
pub struct PluginProcessHost<
    S: ProcessSpawner,
    E: ChildProcessEnvironmentProvider = NoChildProcessEnvironment,
>(Arc<Inner<S, E>>);

impl<S: ProcessSpawner, E: ChildProcessEnvironmentProvider> Clone for PluginProcessHost<S, E> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<S> PluginProcessHost<S, NoChildProcessEnvironment>
where
    S: ProcessSpawner + Send + Sync + 'static,
    S::Process: Send + Sync + 'static,
{
    /// Binds the handler to the plugin it serves, that plugin's package root, and its spawner.
    ///
    /// The package root is bound here, at launch, for the same reason the plugin identity is: it
    /// decides which tree a `packageCommand` may resolve inside, and taking it from a request
    /// would let a plugin name any directory on the host.
    pub fn new(plugin_id: impl Into<String>, package_root: PathBuf, spawner: S) -> Self {
        Self::with_environment_provider(plugin_id, package_root, spawner, NoChildProcessEnvironment)
    }
}

impl<S, E> PluginProcessHost<S, E>
where
    S: ProcessSpawner + Send + Sync + 'static,
    S::Process: Send + Sync + 'static,
    E: ChildProcessEnvironmentProvider,
{
    /// Binds a host-owned environment policy to one plugin process generation.
    pub fn with_environment_provider(
        plugin_id: impl Into<String>,
        package_root: PathBuf,
        spawner: S,
        environment_provider: E,
    ) -> Self {
        let (runtime, runtime_rx) = watch::channel(None);
        Self(Arc::new(Inner {
            plugin_id: plugin_id.into(),
            package_root,
            spawner,
            environment_provider,
            next_id: AtomicU64::new(1),
            tracked: StdMutex::new(HashMap::new()),
            runtime,
            _runtime_rx: runtime_rx,
        }))
    }

    /// Supplies the plugin runtime handle used to push `stdout`/`stderr`/`exit` notifications.
    ///
    /// Called once, right after the launch that used this handler as its `host_requests`
    /// returns: the handler must exist before that launch (it is passed into it), so the runtime
    /// handle it needs in order to talk back to the plugin can only arrive after the fact.
    pub fn attach_runtime(&self, runtime: ProcessPluginRuntime) {
        let _ = self.0.runtime.send(Some(runtime));
    }

    /// Kills every process this handler is still tracking, best effort.
    ///
    /// Called once the plugin generation this handler serves has stopped for any reason —
    /// intentional stop, uninstall, restart, or failure — so a host-spawned process never outlives
    /// the plugin that asked for it.
    pub async fn kill_all(&self) {
        let processes: Vec<Arc<S::Process>> = self
            .0
            .tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|tracked| Arc::clone(&tracked.process))
            .collect();
        for process in processes {
            if let Err(error) = process.kill().await {
                ora_warn!(
                    plugin_id = %self.0.plugin_id,
                    error = %error,
                    "failed to kill a host-managed child process during plugin teardown"
                );
            }
        }
    }

    /// Pushes one notification to the plugin, waiting for `attach_runtime` if it has not run yet.
    ///
    /// The wait only ever blocks for the brief window between a launch returning and its caller
    /// calling `attach_runtime`; once that call lands every later push observes it immediately.
    async fn push(&self, method: &str, params: Value) {
        let mut receiver = self.0.runtime.subscribe();
        if receiver.borrow().is_none() && receiver.changed().await.is_err() {
            return;
        }
        let runtime = receiver.borrow().clone();
        if let Some(runtime) = runtime {
            let _ = runtime.notify(method, params).await;
        }
    }

    /// Resolves one spawn request's program into the exact path handed to the operating system.
    ///
    /// A `packageCommand` is joined onto this plugin's install root and canonicalized, which
    /// rejects a target that escapes the package through a symlink and answers one the package
    /// does not carry with its own classification. The resulting absolute path frees the request's
    /// `cwd` to be the workspace the child should run in, instead of doubling as the directory the
    /// program is resolved against.
    fn resolve_program(&self, program: SpawnProgram) -> Result<PathBuf, ChildProcessError> {
        let relative = match program {
            SpawnProgram::Host(command) => return Ok(PathBuf::from(command)),
            SpawnProgram::Package(relative) => relative,
        };
        let root = CanonicalPathRoot::new(&self.0.package_root).map_err(|error| {
            ChildProcessError::new(
                ChildProcessErrorKind::InvalidPackageCommand,
                format!("the plugin package root is unavailable: {error}"),
            )
        })?;
        // "The package does not carry this file" is kept apart from every other resolution
        // failure because they call for opposite reactions. One plugin source is built into both a
        // package that bundles its CLI and one that does not, and only the host can tell the
        // plugin which it is running from; a missing file is that answer, and the plugin falls
        // back to a PATH lookup on it. Anything else means the package does carry something at
        // that path but it cannot be run, which fails identically on every retry.
        let resolved = root
            .resolve_existing(&relative)
            .map_err(|error| match error {
                PathContainmentError::PathNotFound { .. } => ChildProcessError::new(
                    ChildProcessErrorKind::PackageCommandMissing,
                    format!(
                        "packageCommand `{}` is not part of this plugin package",
                        relative.as_str()
                    ),
                ),
                other => ChildProcessError::new(
                    ChildProcessErrorKind::InvalidPackageCommand,
                    format!(
                        "packageCommand `{}` must resolve inside the plugin package: {other}",
                        relative.as_str()
                    ),
                ),
            })?;
        // Path-based like the containment check above: it cannot prevent a replacement between
        // this check and the spawn, only a package that is already shaped wrong.
        if !resolved.is_file() {
            return Err(ChildProcessError::new(
                ChildProcessErrorKind::InvalidPackageCommand,
                format!(
                    "packageCommand `{}` must name a regular package file",
                    relative.as_str()
                ),
            ));
        }
        Ok(resolved)
    }

    async fn handle_spawn(&self, params: Value) -> Result<Value, HostRequestError> {
        let request = parse_spawn_params(&params)?;
        let program = self.resolve_program(request.program)?;
        let mut spec = ProcessSpec::new(program)
            .args(request.args)
            .stdin(ProcessStdio::Piped)
            .stdout(ProcessStdio::Piped)
            .stderr(ProcessStdio::Piped);
        if request
            .env
            .iter()
            .any(|(key, _)| key.starts_with(MCP_ENVIRONMENT_PREFIX))
        {
            return Err(ChildProcessError::new(
                ChildProcessErrorKind::InvalidParams,
                format!(
                    "environment variable names beginning with `{MCP_ENVIRONMENT_PREFIX}` are reserved"
                ),
            )
            .into());
        }
        let host_environment = match &request.cwd {
            Some(workspace_root) => self
                .0
                .environment_provider
                .environment(&self.0.plugin_id, workspace_root)
                .map_err(|message| ChildProcessError::new(ChildProcessErrorKind::Io, message))?,
            None => BTreeMap::new(),
        };
        if let Some(cwd) = request.cwd {
            spec = spec.cwd(cwd);
        }
        for (key, value) in request.env {
            spec = spec.env(key, value);
        }
        for (key, value) in host_environment {
            spec = spec.env(key, value);
        }

        let mut process = self
            .0
            .spawner
            .spawn(spec)
            .map_err(ChildProcessError::from_spawn_io)?;
        let pid = process.id();
        let stdio = (
            process.take_stdin(),
            process.take_stdout(),
            process.take_stderr(),
        );
        let (Some(stdin), Some(stdout), Some(stderr)) = stdio else {
            let _ = process.kill().await;
            let _ = process.wait().await;
            return Err(ChildProcessError::new(
                ChildProcessErrorKind::Io,
                "spawned process stdio is unavailable",
            )
            .into());
        };

        let process_id = self.0.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let (stdin_tx, stdin_rx) = mpsc::channel(32);
        tokio::spawn(run_stdin_writer(stdin, stdin_rx));

        let process = Arc::new(process);
        let stdout_done = tokio::spawn(pump_output(
            self.clone(),
            process_id.clone(),
            stdout,
            CHILDPROCESS_STDOUT_METHOD,
        ));
        let stderr_done = tokio::spawn(pump_output(
            self.clone(),
            process_id.clone(),
            stderr,
            CHILDPROCESS_STDERR_METHOD,
        ));
        self.0
            .tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                process_id.clone(),
                Tracked {
                    process: Arc::clone(&process),
                    stdin_tx,
                    stdout_done,
                    stderr_done,
                },
            );

        tokio::spawn(watch_exit(self.clone(), process_id.clone(), process));

        Ok(json!(ChildProcessSpawnResult { process_id, pid }))
    }

    async fn handle_write(&self, params: Value) -> Result<Value, HostRequestError> {
        let request: ChildProcessWriteParams = serde_json::from_value(params).map_err(|error| {
            ChildProcessError::new(
                ChildProcessErrorKind::InvalidParams,
                format!("invalid write params: {error}"),
            )
        })?;
        let process_id = request.process_id;
        let bytes_base64 = request.bytes_base64;
        if bytes_base64.len() > MAX_WRITE_BASE64_LEN {
            return Err(ChildProcessError::new(
                ChildProcessErrorKind::InvalidParams,
                format!("bytesBase64 decodes to more than {MAX_WRITE_BYTES} bytes"),
            )
            .into());
        }
        let bytes = BASE64.decode(&bytes_base64).map_err(|error| {
            ChildProcessError::new(
                ChildProcessErrorKind::InvalidParams,
                format!("bytesBase64 is not valid base64: {error}"),
            )
        })?;
        self.send_stdin(&process_id, StdinCommand::Write(bytes))
            .await?;
        Ok(json!({}))
    }

    async fn handle_close_stdin(&self, params: Value) -> Result<Value, HostRequestError> {
        let process_id = required_process_id(&params)?;
        self.send_stdin(&process_id, StdinCommand::Close).await?;
        Ok(json!({}))
    }

    async fn send_stdin(
        &self,
        process_id: &str,
        command: StdinCommand,
    ) -> Result<(), ChildProcessError> {
        let stdin_tx = self
            .0
            .tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(process_id)
            .map(|tracked| tracked.stdin_tx.clone())
            .ok_or_else(|| {
                ChildProcessError::new(ChildProcessErrorKind::NotFound, "unknown processId")
            })?;
        stdin_tx.send(command).await.map_err(|_| {
            ChildProcessError::new(ChildProcessErrorKind::Io, "process stdin is closed")
        })
    }

    async fn handle_kill(&self, params: Value) -> Result<Value, HostRequestError> {
        let process_id = required_process_id(&params)?;
        let process = self
            .0
            .tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&process_id)
            .map(|tracked| Arc::clone(&tracked.process))
            .ok_or_else(|| {
                ChildProcessError::new(ChildProcessErrorKind::NotFound, "unknown processId")
            })?;
        process.kill().await.map_err(|error| {
            ChildProcessError::new(ChildProcessErrorKind::Io, error.to_string())
        })?;
        Ok(json!({}))
    }
}

impl<S, E> HostRequestHandler for PluginProcessHost<S, E>
where
    S: ProcessSpawner + Send + Sync + 'static,
    S::Process: Send + Sync + 'static,
    E: ChildProcessEnvironmentProvider,
{
    async fn handle(&self, method: &str, params: Value) -> Result<Value, HostRequestError> {
        match method {
            CHILDPROCESS_SPAWN_METHOD => self.handle_spawn(params).await,
            CHILDPROCESS_WRITE_METHOD => self.handle_write(params).await,
            CHILDPROCESS_CLOSE_STDIN_METHOD => self.handle_close_stdin(params).await,
            CHILDPROCESS_KILL_METHOD => self.handle_kill(params).await,
            other => Err(HostRequestError::method_not_found(other)),
        }
    }
}

/// Feeds one spawned process's stdin from the channel `write`/`closeStdin` requests publish to.
async fn run_stdin_writer<W: AsyncWrite + Unpin>(
    mut stdin: W,
    mut commands: mpsc::Receiver<StdinCommand>,
) {
    while let Some(command) = commands.recv().await {
        match command {
            StdinCommand::Write(bytes) => {
                if stdin.write_all(&bytes).await.is_err() {
                    return;
                }
            }
            StdinCommand::Close => {
                let _ = stdin.shutdown().await;
                return;
            }
        }
    }
}

/// Forwards every chunk read from one spawned process's stdout or stderr as a notification.
async fn pump_output<S, E, R>(
    host: PluginProcessHost<S, E>,
    process_id: String,
    mut reader: R,
    method: &'static str,
) where
    S: ProcessSpawner + Send + Sync + 'static,
    S::Process: Send + Sync + 'static,
    E: ChildProcessEnvironmentProvider,
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; READ_CHUNK_BYTES];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => return,
            Ok(length) => {
                host.push(
                    method,
                    json!(ChildProcessOutput {
                        process_id: process_id.clone(),
                        bytes_base64: BASE64.encode(&buffer[..length]),
                    }),
                )
                .await;
            }
            Err(_) => return,
        }
    }
}

/// Waits for one spawned process to exit, then reports it and stops tracking it.
async fn watch_exit<S, E>(
    host: PluginProcessHost<S, E>,
    process_id: String,
    process: Arc<S::Process>,
) where
    S: ProcessSpawner + Send + Sync + 'static,
    S::Process: Send + Sync + 'static,
    E: ChildProcessEnvironmentProvider,
{
    let status = process.wait().await;
    let tracked = host
        .0
        .tracked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&process_id);
    // Drain whatever stdout/stderr the pumps already read before announcing exit, so the plugin
    // never sees the exit notification arrive ahead of the process's last output.
    if let Some(tracked) = tracked {
        let _ = tokio::join!(tracked.stdout_done, tracked.stderr_done);
    }
    if let Err(error) = &status {
        ora_warn!(
            plugin_id = %host.0.plugin_id,
            error = %error,
            "failed to wait for a host-managed child process"
        );
    }
    let (code, signal) = exit_fields(status.as_ref());
    host.push(
        CHILDPROCESS_EXIT_METHOD,
        json!(ChildProcessExit {
            process_id,
            code,
            signal,
        }),
    )
    .await;
}

/// Splits a process's final status into a wire-friendly exit code and, on Unix, a signal number.
fn exit_fields(status: Result<&ExitStatus, &io::Error>) -> (Option<i32>, Option<i32>) {
    let Ok(status) = status else {
        return (None, None);
    };
    let code = status.code();
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    (code, signal)
}

/// Names the executable one spawn request wants, and therefore who resolves it.
enum SpawnProgram {
    /// Resolved by the operating system: a PATH lookup, or a path the plugin already knows.
    Host(String),
    /// Resolved by the host against the calling plugin's own package root.
    Package(PortableRelativePath),
}

/// One validated `spawn` request.
struct SpawnParams {
    program: SpawnProgram,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
}

/// Parses and validates the `spawn` params, rejecting anything not shaped like the documented
/// `{ command | packageCommand, args?, cwd?, env? }`.
fn parse_spawn_params(params: &Value) -> Result<SpawnParams, ChildProcessError> {
    let request: ChildProcessSpawnParams =
        serde_json::from_value(params.clone()).map_err(|error| {
            ChildProcessError::new(
                ChildProcessErrorKind::InvalidParams,
                format!("invalid spawn params: {error}"),
            )
        })?;
    let program = parse_spawn_program(request.command, request.package_command)?;
    Ok(SpawnParams {
        program,
        args: request.args,
        cwd: request.cwd.map(PathBuf::from),
        env: request.env.into_iter().collect(),
    })
}

/// Selects the one program field a spawn request carries.
///
/// The two forms are mutually exclusive rather than one falling back to the other: a relative
/// `command` and a `packageCommand` are indistinguishable as strings, so letting one field serve
/// both meanings would make the callsite decide by accident which directory resolves it.
fn parse_spawn_program(
    command: Option<String>,
    package_command: Option<String>,
) -> Result<SpawnProgram, ChildProcessError> {
    match (command, package_command) {
        (Some(_), Some(_)) => Err(ChildProcessError::new(
            ChildProcessErrorKind::InvalidParams,
            "command and packageCommand are mutually exclusive",
        )),
        (None, None) => Err(ChildProcessError::new(
            ChildProcessErrorKind::InvalidParams,
            "missing string command or packageCommand",
        )),
        (Some(command), None) => {
            if command.trim().is_empty() {
                return Err(ChildProcessError::new(
                    ChildProcessErrorKind::InvalidCommand,
                    "command must not be empty",
                ));
            }
            Ok(SpawnProgram::Host(command))
        }
        // Portable parsing is what makes the value safe to join: it rejects parent traversal,
        // rooted paths, drive and UNC prefixes, reserved device names, and NUL on every host, so
        // the same package behaves identically wherever it is installed.
        (None, Some(package_command)) => PortableRelativePath::parse(&package_command)
            .map(SpawnProgram::Package)
            .map_err(|error| {
                ChildProcessError::new(
                    ChildProcessErrorKind::InvalidPackageCommand,
                    format!("packageCommand is not a portable package-relative path: {error}"),
                )
            }),
    }
}

/// Extracts and validates the `processId` param shared by `write`, `closeStdin`, and `kill`.
fn required_process_id(params: &Value) -> Result<String, ChildProcessError> {
    serde_json::from_value::<ChildProcessIdParams>(params.clone())
        .map(|request| request.process_id)
        .map_err(|error| {
            ChildProcessError::new(
                ChildProcessErrorKind::InvalidParams,
                format!("invalid process params: {error}"),
            )
        })
}
