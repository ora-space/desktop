//! Tests for `ora/childprocess/*`: request handling and tracking lifecycle against a fake spawned
//! process, plus one end-to-end check that stdout, stderr, and exit reach the plugin as
//! notifications once a real `PluginRuntime` is attached.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ora_plugin_runtime::{
    HostRequestError, HostRequestHandler, NoHostRequests, PluginRuntime as ProcessPluginRuntime,
    PluginRuntimeConfig,
};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

use crate::childprocess::{
    CHILDPROCESS_CLOSE_STDIN_METHOD, CHILDPROCESS_KILL_METHOD, CHILDPROCESS_SPAWN_METHOD,
    CHILDPROCESS_WRITE_METHOD, ChildProcessEnvironmentProvider, MAX_WRITE_BYTES, PluginProcessHost,
};

#[derive(Clone)]
struct FixedEnvironmentProvider;

impl ChildProcessEnvironmentProvider for FixedEnvironmentProvider {
    fn environment(
        &self,
        plugin_id: &str,
        workspace_root: &Path,
    ) -> Result<BTreeMap<String, String>, String> {
        assert_eq!(plugin_id, "plugin-a");
        assert_eq!(workspace_root, Path::new("/work"));
        Ok(BTreeMap::from([(
            "HOST_INJECTED".to_string(),
            "secret".to_string(),
        )]))
    }
}

/// The `data.kind` classification of one failed call, as a plugin would branch on it.
fn kind_of(error: &HostRequestError) -> String {
    error.data()["kind"].as_str().unwrap_or_default().to_owned()
}

// ---------------------------------------------------------------------------------------------
// Fake spawned child process: stands in for the OS process behind `ora/childprocess/spawn`.
// ---------------------------------------------------------------------------------------------

/// The test-facing peer of one spawned [`FakeChildProcess`]: the other end of every piped stream.
struct FakeChildTestHandle {
    stdin_peer: DuplexStream,
    stdout_peer: DuplexStream,
    stderr_peer: DuplexStream,
    exit_tx: watch::Sender<Option<ExitStatus>>,
    killed: Arc<AtomicBool>,
}

#[derive(Clone)]
struct FakeChildSpawner(Arc<FakeChildSpawnerState>);

struct FakeChildSpawnerState {
    next_id: AtomicU32,
    calls: StdMutex<Vec<ProcessSpec>>,
    handles: StdMutex<Vec<FakeChildTestHandle>>,
    force_next_error: StdMutex<Option<io::ErrorKind>>,
}

impl FakeChildSpawner {
    fn new() -> Self {
        Self(Arc::new(FakeChildSpawnerState {
            next_id: AtomicU32::new(100),
            calls: StdMutex::new(Vec::new()),
            handles: StdMutex::new(Vec::new()),
            force_next_error: StdMutex::new(None),
        }))
    }

    /// Makes the next `spawn` call fail as if the OS could not start the process.
    fn force_next_error(&self, kind: io::ErrorKind) {
        *self
            .0
            .force_next_error
            .lock()
            .expect("lock force_next_error") = Some(kind);
    }

    /// The `ProcessSpec` passed to every `spawn` call so far, in order.
    fn calls(&self) -> Vec<ProcessSpec> {
        self.0.calls.lock().expect("lock calls").clone()
    }

    /// Takes the test-facing peer of the most recently spawned process.
    fn last_handle(&self) -> FakeChildTestHandle {
        self.0
            .handles
            .lock()
            .expect("lock handles")
            .pop()
            .expect("a process was spawned")
    }
}

impl ProcessSpawner for FakeChildSpawner {
    type Process = FakeChildProcess;

    fn spawn(&self, spec: ProcessSpec) -> io::Result<Self::Process> {
        if let Some(kind) = self
            .0
            .force_next_error
            .lock()
            .expect("lock force_next_error")
            .take()
        {
            return Err(io::Error::from(kind));
        }
        self.0.calls.lock().expect("lock calls").push(spec);
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let (stdin_host, stdin_peer) = tokio::io::duplex(8192);
        let (stdout_peer, stdout_host) = tokio::io::duplex(8192);
        let (stderr_peer, stderr_host) = tokio::io::duplex(8192);
        let (exit_tx, _) = watch::channel(None);
        let killed = Arc::new(AtomicBool::new(false));
        self.0
            .handles
            .lock()
            .expect("lock handles")
            .push(FakeChildTestHandle {
                stdin_peer,
                stdout_peer,
                stderr_peer,
                exit_tx: exit_tx.clone(),
                killed: Arc::clone(&killed),
            });
        Ok(FakeChildProcess {
            id: Some(id),
            stdin: Some(stdin_host),
            stdout: Some(stdout_host),
            stderr: Some(stderr_host),
            exit_tx,
            killed,
        })
    }
}

struct FakeChildProcess {
    id: Option<u32>,
    stdin: Option<DuplexStream>,
    stdout: Option<DuplexStream>,
    stderr: Option<DuplexStream>,
    exit_tx: watch::Sender<Option<ExitStatus>>,
    killed: Arc<AtomicBool>,
}

impl ManagedProcess for FakeChildProcess {
    type Stdin = DuplexStream;
    type Stdout = DuplexStream;
    type Stderr = DuplexStream;

    fn id(&self) -> Option<u32> {
        self.id
    }

    fn take_stdin(&mut self) -> Option<Self::Stdin> {
        self.stdin.take()
    }

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.stderr.take()
    }

    fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        Ok(*self.exit_tx.borrow())
    }

    async fn wait(&self) -> io::Result<ExitStatus> {
        let mut exited = self.exit_tx.subscribe();
        loop {
            if let Some(status) = *exited.borrow() {
                return Ok(status);
            }
            exited
                .changed()
                .await
                .map_err(|_| io::Error::other("fake process exit channel closed"))?;
        }
    }

    async fn kill(&self) -> io::Result<()> {
        self.killed.store(true, Ordering::Release);
        self.exit_tx.send_replace(Some(exit_status(0)));
        Ok(())
    }
}

fn exit_status(code: i32) -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }
}

// ---------------------------------------------------------------------------------------------
// Handler behavior against the fake spawner: no runtime attached, so pushed notifications are
// never observed here — see `pushes_stdout_stderr_and_exit_once_a_runtime_is_attached` below.
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn spawn_returns_process_id_and_forwards_command_args_cwd_env() {
    let spawner = FakeChildSpawner::new();
    let host = PluginProcessHost::new("plugin-a", PathBuf::from("package-root"), spawner.clone());

    let result = host
        .handle(
            CHILDPROCESS_SPAWN_METHOD,
            json!({
                "command": "opencode",
                "args": ["acp", "--cwd", "/work"],
                "cwd": "/work",
                "env": { "FOO": "bar" },
            }),
        )
        .await
        .expect("spawn succeeds");

    let calls = spawner.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program(), OsStr::new("opencode"));
    assert_eq!(
        calls[0].args_iter().collect::<Vec<_>>(),
        vec![OsStr::new("acp"), OsStr::new("--cwd"), OsStr::new("/work")]
    );
    assert_eq!(calls[0].cwd_path(), Some(Path::new("/work")));
    assert_eq!(
        calls[0].envs().collect::<Vec<_>>(),
        vec![(OsStr::new("FOO"), OsStr::new("bar"))]
    );
    assert_eq!(result, json!({ "processId": "1", "pid": 100 }));
}

#[tokio::test]
async fn spawn_injects_host_environment_after_binding_the_workspace() {
    let spawner = FakeChildSpawner::new();
    let host = PluginProcessHost::with_environment_provider(
        "plugin-a",
        PathBuf::from("package-root"),
        spawner.clone(),
        FixedEnvironmentProvider,
    );

    host.handle(
        CHILDPROCESS_SPAWN_METHOD,
        json!({ "command": "agent", "cwd": "/work", "env": { "PUBLIC": "value" } }),
    )
    .await
    .expect("spawn succeeds");

    assert_eq!(
        spawner.calls()[0].envs().collect::<Vec<_>>(),
        vec![
            (OsStr::new("PUBLIC"), OsStr::new("value")),
            (OsStr::new("HOST_INJECTED"), OsStr::new("secret")),
        ]
    );
}

#[tokio::test]
async fn spawn_rejects_plugin_supplied_reserved_mcp_environment() {
    let spawner = FakeChildSpawner::new();
    let host = PluginProcessHost::new("plugin-a", PathBuf::from("package-root"), spawner.clone());

    let error = host
        .handle(
            CHILDPROCESS_SPAWN_METHOD,
            json!({
                "command": "agent",
                "cwd": "/work",
                "env": { "ORA_MCP_FORGED": "value" },
            }),
        )
        .await
        .expect_err("reserved variables are host-owned");

    assert_eq!(
        (kind_of(&error), spawner.calls().len()),
        ("invalid_params".to_string(), 0)
    );
}

/// Builds a package root holding one executable file, returning the root and its handler.
fn package_with_executable(
    spawner: FakeChildSpawner,
    relative: &str,
) -> (tempfile::TempDir, PluginProcessHost<FakeChildSpawner>) {
    let package = tempfile::TempDir::new().expect("create package root");
    let executable = package.path().join(relative);
    std::fs::create_dir_all(executable.parent().expect("executable has a parent"))
        .expect("create executable directory");
    std::fs::write(&executable, b"binary").expect("write executable");
    let host = PluginProcessHost::new("plugin-a", package.path().to_path_buf(), spawner);
    (package, host)
}

#[tokio::test]
async fn spawn_resolves_package_command_against_the_plugin_package_root() {
    let spawner = FakeChildSpawner::new();
    let (package, host) = package_with_executable(spawner.clone(), "assets/bin/opencode");

    host.handle(
        CHILDPROCESS_SPAWN_METHOD,
        json!({
            "packageCommand": "assets/bin/opencode",
            "args": ["acp"],
            "cwd": "/work",
        }),
    )
    .await
    .expect("spawn succeeds");

    // The program is the absolute path inside the package, while `cwd` stays the workspace the
    // child runs in: the two are independent, which is exactly what a relative `command` cannot do.
    let calls = spawner.calls();
    let expected = package
        .path()
        .join("assets/bin/opencode")
        .canonicalize()
        .expect("canonicalize the bundled executable");
    assert_eq!(
        (
            calls.len(),
            calls[0].program().to_owned(),
            calls[0].cwd_path().map(Path::to_path_buf),
        ),
        (1, expected.into_os_string(), Some(PathBuf::from("/work"))),
    );
}

/// A resolved `packageCommand` is spawnable by the real operating system, not just by a fake.
///
/// Every other test here injects a spawner, so none of them would notice that resolution produces
/// a path the OS refuses. That is a live risk rather than a theoretical one: `resolve_existing`
/// canonicalizes, and on Windows a canonical path is the `\\?\C:\...` verbatim form, which not
/// every process API accepts. A real binary copied into a real package root is what proves the
/// path handed to `ProcessSpec` can actually start.
#[tokio::test]
async fn a_resolved_package_command_is_spawnable_by_the_operating_system() {
    let package = tempfile::TempDir::new().expect("create package root");
    std::fs::create_dir_all(package.path().join("assets/bin")).expect("create bin directory");

    // A binary guaranteed to exist on the host, copied in so it is genuinely package-contained.
    #[cfg(windows)]
    let (source, relative, args) = (
        PathBuf::from(
            std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_string()),
        ),
        "assets/bin/tool.exe",
        vec!["/c", "exit 0"],
    );
    #[cfg(unix)]
    let (source, relative, args) = (
        PathBuf::from("/bin/sh"),
        "assets/bin/tool",
        vec!["-c", "exit 0"],
    );
    std::fs::copy(&source, package.path().join(relative)).expect("copy the host binary");

    let host = PluginProcessHost::new(
        "plugin-a",
        package.path().to_path_buf(),
        ora_process::TokioProcessSpawner::new(),
    );

    let result = host
        .handle(
            CHILDPROCESS_SPAWN_METHOD,
            json!({ "packageCommand": relative, "args": args }),
        )
        .await
        .expect("the resolved package binary spawns");

    // A real pid is the proof: the OS accepted the canonical path and created a process.
    assert!(
        result["pid"].as_u64().is_some_and(|pid| pid > 0),
        "expected a real process id, got {result}"
    );
    host.kill_all().await;
}

#[tokio::test]
async fn spawn_rejects_a_package_command_that_escapes_the_package() {
    let (_package, host) = package_with_executable(FakeChildSpawner::new(), "assets/bin/opencode");

    let error = host
        .handle(
            CHILDPROCESS_SPAWN_METHOD,
            json!({ "packageCommand": "../../../bin/sh" }),
        )
        .await
        .expect_err("a traversing package path is rejected");

    assert_eq!(kind_of(&error), "invalid_package_command");
}

/// A package that does not carry the executable is reported apart from a package that carries a
/// broken one, because a plugin reacts to the two in opposite ways: the first is how a package
/// built without a bundled CLI announces itself, and the plugin answers it by falling back to a
/// PATH lookup, while the second is a deterministic package fault it must not retry.
#[tokio::test]
async fn spawn_reports_a_package_command_the_package_does_not_carry_as_missing() {
    let (_package, host) = package_with_executable(FakeChildSpawner::new(), "assets/bin/opencode");

    let error = host
        .handle(
            CHILDPROCESS_SPAWN_METHOD,
            json!({ "packageCommand": "assets/bin/missing" }),
        )
        .await
        .expect_err("an absent package path is rejected");

    assert_eq!(kind_of(&error), "package_command_missing");
}

#[tokio::test]
async fn spawn_rejects_a_package_command_that_is_not_a_regular_file() {
    let (package, host) = package_with_executable(FakeChildSpawner::new(), "assets/bin/opencode");
    std::fs::create_dir_all(package.path().join("assets/bin/tools"))
        .expect("create a directory where an executable is expected");

    let error = host
        .handle(
            CHILDPROCESS_SPAWN_METHOD,
            json!({ "packageCommand": "assets/bin/tools" }),
        )
        .await
        .expect_err("a package path that is not a regular file is rejected");

    assert_eq!(kind_of(&error), "invalid_package_command");
}

#[tokio::test]
async fn spawn_rejects_naming_both_program_forms_or_neither() {
    let host = PluginProcessHost::new(
        "plugin-a",
        PathBuf::from("package-root"),
        FakeChildSpawner::new(),
    );

    let both = host
        .handle(
            CHILDPROCESS_SPAWN_METHOD,
            json!({ "command": "opencode", "packageCommand": "assets/bin/opencode" }),
        )
        .await
        .expect_err("naming both forms is rejected");
    let neither = host
        .handle(CHILDPROCESS_SPAWN_METHOD, json!({ "args": ["acp"] }))
        .await
        .expect_err("naming neither form is rejected");

    assert_eq!(
        (kind_of(&both), kind_of(&neither)),
        ("invalid_params".to_string(), "invalid_params".to_string()),
    );
}

#[tokio::test]
async fn spawn_rejects_an_empty_command() {
    let host = PluginProcessHost::new(
        "plugin-a",
        PathBuf::from("package-root"),
        FakeChildSpawner::new(),
    );

    let error = host
        .handle(CHILDPROCESS_SPAWN_METHOD, json!({ "command": "   " }))
        .await
        .expect_err("empty command is rejected");

    assert_eq!(kind_of(&error), "invalid_command");
}

#[tokio::test]
async fn spawn_maps_a_missing_executable_to_program_not_found() {
    let spawner = FakeChildSpawner::new();
    spawner.force_next_error(io::ErrorKind::NotFound);
    let host = PluginProcessHost::new("plugin-a", PathBuf::from("package-root"), spawner);

    let error = host
        .handle(CHILDPROCESS_SPAWN_METHOD, json!({ "command": "missing" }))
        .await
        .expect_err("spawn fails");

    assert_eq!(kind_of(&error), "program_not_found");
}

#[tokio::test]
async fn spawn_maps_any_other_spawn_failure_to_io() {
    let spawner = FakeChildSpawner::new();
    spawner.force_next_error(io::ErrorKind::PermissionDenied);
    let host = PluginProcessHost::new("plugin-a", PathBuf::from("package-root"), spawner);

    let error = host
        .handle(CHILDPROCESS_SPAWN_METHOD, json!({ "command": "opencode" }))
        .await
        .expect_err("spawn fails");

    assert_eq!(kind_of(&error), "io");
}

#[tokio::test]
async fn write_forwards_bytes_and_close_stdin_signals_eof() {
    let spawner = FakeChildSpawner::new();
    let host = PluginProcessHost::new("plugin-a", PathBuf::from("package-root"), spawner.clone());
    let spawned = host
        .handle(CHILDPROCESS_SPAWN_METHOD, json!({ "command": "opencode" }))
        .await
        .expect("spawn succeeds");
    let process_id = spawned["processId"]
        .as_str()
        .expect("processId is a string")
        .to_owned();
    let mut test_handle = spawner.last_handle();

    host.handle(
        CHILDPROCESS_WRITE_METHOD,
        json!({ "processId": process_id, "bytesBase64": BASE64.encode(b"hello\n") }),
    )
    .await
    .expect("write succeeds");

    let mut buffer = [0_u8; 16];
    let read = timeout(
        Duration::from_secs(1),
        test_handle.stdin_peer.read(&mut buffer),
    )
    .await
    .expect("stdin write arrives before timeout")
    .expect("stdin read succeeds");
    assert_eq!(&buffer[..read], b"hello\n");

    host.handle(
        CHILDPROCESS_CLOSE_STDIN_METHOD,
        json!({ "processId": process_id }),
    )
    .await
    .expect("close stdin succeeds");

    let eof = timeout(
        Duration::from_secs(1),
        test_handle.stdin_peer.read(&mut buffer),
    )
    .await
    .expect("eof arrives before timeout")
    .expect("stdin read succeeds");
    assert_eq!(eof, 0);
}

#[tokio::test]
async fn write_rejects_a_payload_over_the_size_limit_before_decoding_it() {
    let host = PluginProcessHost::new(
        "plugin-a",
        PathBuf::from("package-root"),
        FakeChildSpawner::new(),
    );
    // Longer than any base64 string that could decode to `MAX_WRITE_BYTES`; the content does not
    // need to be valid base64 because the size check runs before `BASE64.decode`.
    let oversized = "A".repeat(MAX_WRITE_BYTES.div_ceil(3) * 4 + 4);

    let error = host
        .handle(
            CHILDPROCESS_WRITE_METHOD,
            json!({ "processId": "missing", "bytesBase64": oversized }),
        )
        .await
        .expect_err("oversized payload is rejected");

    assert_eq!(kind_of(&error), "invalid_params");
}

#[tokio::test]
async fn operations_on_an_unknown_process_id_are_not_found() {
    let host = PluginProcessHost::new(
        "plugin-a",
        PathBuf::from("package-root"),
        FakeChildSpawner::new(),
    );

    for (method, params) in [
        (
            CHILDPROCESS_WRITE_METHOD,
            json!({ "processId": "missing", "bytesBase64": "" }),
        ),
        (
            CHILDPROCESS_CLOSE_STDIN_METHOD,
            json!({ "processId": "missing" }),
        ),
        (CHILDPROCESS_KILL_METHOD, json!({ "processId": "missing" })),
    ] {
        let error = host.handle(method, params).await.expect_err(method);
        assert_eq!(kind_of(&error), "not_found", "method {method}");
    }
}

#[tokio::test]
async fn kill_terminates_the_process_which_then_stops_being_tracked() {
    let spawner = FakeChildSpawner::new();
    let host = PluginProcessHost::new("plugin-a", PathBuf::from("package-root"), spawner.clone());
    let spawned = host
        .handle(CHILDPROCESS_SPAWN_METHOD, json!({ "command": "opencode" }))
        .await
        .expect("spawn succeeds");
    let process_id = spawned["processId"]
        .as_str()
        .expect("processId is a string")
        .to_owned();
    let test_handle = spawner.last_handle();

    host.handle(
        CHILDPROCESS_KILL_METHOD,
        json!({ "processId": process_id.clone() }),
    )
    .await
    .expect("kill succeeds");
    assert!(test_handle.killed.load(Ordering::Acquire));

    // The exit-watcher task removes the tracking entry once `wait()` observes the exit `kill()`
    // triggered; poll for that instead of sleeping a fixed duration.
    timeout(Duration::from_secs(1), async {
        loop {
            let result = host
                .handle(CHILDPROCESS_KILL_METHOD, json!({ "processId": process_id }))
                .await;
            if matches!(&result, Err(error) if kind_of(error) == "not_found") {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("process becomes untracked once it has exited");
}

#[tokio::test]
async fn kill_all_terminates_every_tracked_process() {
    let spawner = FakeChildSpawner::new();
    let host = PluginProcessHost::new("plugin-a", PathBuf::from("package-root"), spawner.clone());
    host.handle(CHILDPROCESS_SPAWN_METHOD, json!({ "command": "one" }))
        .await
        .expect("spawn succeeds");
    host.handle(CHILDPROCESS_SPAWN_METHOD, json!({ "command": "two" }))
        .await
        .expect("spawn succeeds");
    let second = spawner.last_handle();
    let first = spawner.last_handle();

    host.kill_all().await;

    assert_eq!(
        (
            first.killed.load(Ordering::Acquire),
            second.killed.load(Ordering::Acquire)
        ),
        (true, true)
    );
}

// ---------------------------------------------------------------------------------------------
// End-to-end: stdout, stderr, and exit reach the plugin as `ora/childprocess/*` notifications
// once a real `PluginRuntime` is attached, exercising `attach_runtime`'s late-binding wait.
// ---------------------------------------------------------------------------------------------

const JSON_RPC_FRAME_TYPE: u8 = 0x01;

/// Writes one frame using the same length-delimited envelope `ora-plugin-runtime` speaks; this is
/// a standalone reimplementation for the test's "fake plugin" side, not an import of the crate's
/// private codec.
async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) {
    let length = u32::try_from(payload.len() + 1).expect("payload fits in a frame");
    writer
        .write_all(&length.to_be_bytes())
        .await
        .expect("write frame header");
    writer
        .write_all(&[JSON_RPC_FRAME_TYPE])
        .await
        .expect("write frame type");
    writer
        .write_all(payload)
        .await
        .expect("write frame payload");
    writer.flush().await.expect("flush frame");
}

/// Reads one frame using the same envelope; returns `None` at a clean EOF.
async fn read_frame<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Option<Vec<u8>> {
    let mut header = [0_u8; 4];
    if reader.read_exact(&mut header).await.is_err() {
        return None;
    }
    let length = u32::from_be_bytes(header) as usize;
    let mut frame = vec![0_u8; length];
    reader
        .read_exact(&mut frame)
        .await
        .expect("read frame body");
    assert_eq!(frame[0], JSON_RPC_FRAME_TYPE);
    Some(frame.split_off(1))
}

/// A `ManagedProcess` standing in for the plugin's own Deno process, so a real `PluginRuntime` can
/// be constructed against a duplex pipe instead of a real subprocess.
struct FakePluginProcess {
    stdin: Option<DuplexStream>,
    stdout: Option<DuplexStream>,
    stderr: Option<DuplexStream>,
}

impl ManagedProcess for FakePluginProcess {
    type Stdin = DuplexStream;
    type Stdout = DuplexStream;
    type Stderr = DuplexStream;

    fn id(&self) -> Option<u32> {
        Some(1)
    }

    fn take_stdin(&mut self) -> Option<Self::Stdin> {
        self.stdin.take()
    }

    fn take_stdout(&mut self) -> Option<Self::Stdout> {
        self.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<Self::Stderr> {
        self.stderr.take()
    }

    fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        Ok(None)
    }

    async fn wait(&self) -> io::Result<ExitStatus> {
        std::future::pending().await
    }

    async fn kill(&self) -> io::Result<()> {
        Ok(())
    }
}

/// Spawns exactly one pre-built [`FakePluginProcess`], ignoring the `ProcessSpec`.
struct SinglePluginProcessSpawner(StdMutex<Option<FakePluginProcess>>);

impl ProcessSpawner for SinglePluginProcessSpawner {
    type Process = FakePluginProcess;

    fn spawn(&self, _spec: ProcessSpec) -> io::Result<Self::Process> {
        Ok(self
            .0
            .lock()
            .expect("lock plugin process")
            .take()
            .expect("spawn called once"))
    }
}

#[tokio::test]
async fn pushes_stdout_before_exit_even_when_the_process_exits_immediately_after_writing() {
    let entrypoint = tempfile::NamedTempFile::new().expect("create fake entrypoint file");

    let (host_stdin, mut plugin_reads_from_host) = tokio::io::duplex(8192);
    let (mut plugin_writes_to_host, host_stdout) = tokio::io::duplex(8192);
    let (_plugin_stderr_peer, host_stderr) = tokio::io::duplex(8192);
    let plugin_spawner = SinglePluginProcessSpawner(StdMutex::new(Some(FakePluginProcess {
        stdin: Some(host_stdin),
        stdout: Some(host_stdout),
        stderr: Some(host_stderr),
    })));

    let (frames_tx, mut frames_rx) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        write_frame(
            &mut plugin_writes_to_host,
            br#"{"jsonrpc":"2.0","method":"ora/register","params":{"methods":[],"emits":[]}}"#,
        )
        .await;
        while let Some(payload) = read_frame(&mut plugin_reads_from_host).await {
            let value: Value = serde_json::from_slice(&payload).expect("host frame is valid JSON");
            if frames_tx.send(value).is_err() {
                return;
            }
        }
    });

    let (runtime, _notifications) = ProcessPluginRuntime::launch(
        &plugin_spawner,
        PluginRuntimeConfig {
            plugin_id: "plugin-a".to_string(),
            deno_path: PathBuf::from("deno"),
            entrypoint: entrypoint.path().to_path_buf(),
            permissions: Vec::new(),
            cwd: None,
            ready_timeout: Duration::from_secs(5),
            call_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(5),
        },
        NoHostRequests,
    )
    .await
    .expect("fake plugin runtime launches");

    let child_spawner = FakeChildSpawner::new();
    let processes = PluginProcessHost::new(
        "plugin-a",
        PathBuf::from("package-root"),
        child_spawner.clone(),
    );
    processes.attach_runtime(runtime);

    let spawned = processes
        .handle(CHILDPROCESS_SPAWN_METHOD, json!({ "command": "opencode" }))
        .await
        .expect("spawn succeeds");
    let process_id = spawned["processId"]
        .as_str()
        .expect("processId is a string")
        .to_owned();
    let mut child_test_handle = child_spawner.last_handle();

    child_test_handle
        .stdout_peer
        .write_all(b"chunk-one")
        .await
        .expect("write fake stdout chunk");

    // Fire the exit signal immediately after the write, without waiting for the stdout
    // notification to be observed first: this is the interleaving that let `watch_exit`
    // race ahead of `pump_output` and reorder the notifications before the join fix. A real OS
    // process closes its stdout/stderr pipes as part of exiting, which is what lets
    // `pump_output` observe EOF and finish; drop the fake's write ends the same way, or the exit
    // notification would wait on `watch_exit`'s join forever.
    let FakeChildTestHandle {
        stdout_peer,
        stderr_peer,
        exit_tx,
        ..
    } = child_test_handle;
    drop(stdout_peer);
    drop(stderr_peer);
    exit_tx.send_replace(Some(exit_status(0)));

    let stdout_notification = timeout(Duration::from_secs(2), frames_rx.recv())
        .await
        .expect("stdout notification arrives before timeout")
        .expect("frame channel stays open");
    assert_eq!(
        stdout_notification,
        json!({
            "jsonrpc": "2.0",
            "method": "ora/childprocess/stdout",
            "params": { "processId": process_id, "bytesBase64": BASE64.encode(b"chunk-one") },
        })
    );

    let exit_notification = timeout(Duration::from_secs(2), frames_rx.recv())
        .await
        .expect("exit notification arrives before timeout")
        .expect("frame channel stays open");
    assert_eq!(
        exit_notification,
        json!({
            "jsonrpc": "2.0",
            "method": "ora/childprocess/exit",
            "params": { "processId": process_id, "code": 0, "signal": Value::Null },
        })
    );
}
