#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Executable-level composition tests. A child-only stand-in replaces `ora-node` at the process
//! boundary so readiness, stop and crash behavior can be shaped without Git, host or guardian.
use ora_contracts::controller_api::*;
use ora_controller::{CloneIntake, SqliteStore};
use ora_node_protocol::{ControllerId, RequestId};
use ora_utils::process::{LinuxPidFd, ProcessSignal, linux_process};
use pretty_assertions::assert_eq;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// Shapes the stand-in Node's behavior; the modes mirror the failure paths the ADR names.
#[derive(Clone, Copy)]
enum FakeNode {
    /// Binds the endpoint and exits cleanly on SIGTERM, like the real Node.
    Listen,
    /// Never binds the endpoint, so readiness cannot be observed.
    Silent,
    /// Binds the endpoint but ignores SIGTERM, so the stop deadline expires.
    Stubborn,
}

extern "C" fn exit_cleanly(_: libc::c_int) {
    // SAFETY: only an async-signal-safe exit runs in the handler.
    unsafe { libc::_exit(0) }
}

/// Child-only fixture started through the shell wrapper `deployment()` writes; never run directly.
#[test]
#[ignore = "subprocess fixture, invoked explicitly by composition tests"]
fn fake_node_child() {
    let endpoint = PathBuf::from(std::env::var_os("ORA_TEST_FAKE_NODE_ENDPOINT").unwrap());
    let mode = std::env::var("ORA_TEST_FAKE_NODE_MODE").unwrap();
    fs::write(
        endpoint.with_file_name("fake-node.pid"),
        std::process::id().to_string(),
    )
    .unwrap();
    // SAFETY: installing a process-wide disposition before any other thread exists in this child.
    unsafe {
        match mode.as_str() {
            "stubborn" => libc::signal(libc::SIGTERM, libc::SIG_IGN),
            _ => libc::signal(
                libc::SIGTERM,
                exit_cleanly as *const () as libc::sighandler_t,
            ),
        };
    }
    if mode != "silent" {
        let listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
        // Hold accepted connections open so a Controller session blocks instead of seeing EOF.
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming().flatten() {
                held.push(stream);
            }
        });
    }
    loop {
        std::thread::sleep(Duration::from_secs(/*secs*/ 1));
    }
}

struct Deployment {
    root: tempfile::TempDir,
    config: PathBuf,
    home: PathBuf,
    node_config: PathBuf,
    endpoint: PathBuf,
}

/// Writes a complete hosted deployment whose Node is the stand-in in the requested mode.
fn deployment(mode: FakeNode, ready_timeout_ms: u64, stop_timeout_ms: u64) -> Deployment {
    let root = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
        .tempdir_in(std::env::var_os("HOME").unwrap())
        .unwrap();
    let path = root.path();
    fs::create_dir(path.join("node")).unwrap();
    let endpoint = path.join("node").join("control.sock");
    let node_config = path.join("node.json");
    fs::write(
        &node_config,
        serde_json::to_vec(&serde_json::json!({
            "control": { "controller_id": "owner", "listen": { "kind": "ipc", "path": endpoint } }
        }))
        .unwrap(),
    )
    .unwrap();
    let wrapper = path.join("fake-node.sh");
    let mode = match mode {
        FakeNode::Listen => "listen",
        FakeNode::Silent => "silent",
        FakeNode::Stubborn => "stubborn",
    };
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec env ORA_TEST_FAKE_NODE_ENDPOINT='{}' ORA_TEST_FAKE_NODE_MODE='{mode}' '{}' --ignored --exact fake_node_child\n",
            endpoint.display(),
            std::env::current_exe().unwrap().display()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(/*mode*/ 0o700)).unwrap();
    let home = path.join("controller");
    let config = path.join("controller.json");
    fs::write(
        &config,
        serde_json::to_vec(&serde_json::json!({
            "controller": {
                "home_directory": home,
                "persistence": { "kind": "sqlite" },
                "protected_state_directories": [path.join("node")],
                "controller_id": "owner",
                "nodes": [{ "node_id": "node", "endpoint": { "kind": "ipc", "path": endpoint } }],
                "session": { "io_timeout_ms": 500, "query_interval_ms": 100 },
                "reconnect_ms": 200,
                "timezone": "Asia/Shanghai",
            },
            "api": { "node_id": "node" },
            "single_node": {
                "node_executable": wrapper,
                "node_config": node_config,
                "ready_timeout_ms": ready_timeout_ms,
                "stop_timeout_ms": stop_timeout_ms,
            },
        }))
        .unwrap(),
    )
    .unwrap();
    Deployment {
        root,
        config,
        home,
        node_config,
        endpoint,
    }
}

struct Executable {
    child: Child,
    log: PathBuf,
    errors: PathBuf,
}
impl Executable {
    fn start(root: &Path, args: &[&str]) -> Self {
        let log = root.join("controller.log");
        let errors = root.join("controller.err");
        let child = Command::new(env!("CARGO_BIN_EXE_ora-controller"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(fs::File::create(&log).unwrap())
            .stderr(fs::File::create(&errors).unwrap())
            .spawn()
            .unwrap();
        Self { child, log, errors }
    }
    fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
    fn errors(&self) -> String {
        fs::read_to_string(&self.errors).unwrap_or_default()
    }
    /// Waits for the actual bound address; the executable logs it only after its Node is ready.
    fn address(&self) -> String {
        let mut address = None;
        until(|| {
            address = listening(&self.log());
            address.is_some()
        });
        address.unwrap()
    }
    fn wait(&mut self) -> std::process::ExitStatus {
        until(|| self.child.try_wait().unwrap().is_some());
        self.child.wait().unwrap()
    }
    fn terminate(&mut self) -> std::process::ExitStatus {
        LinuxPidFd::for_child(&self.child)
            .unwrap()
            .signal(ProcessSignal::Terminate)
            .unwrap();
        self.wait()
    }
}
impl Drop for Executable {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Pins the stand-in Node through its pid file; the Controller, not the test, is its parent.
fn fake_node(deployment: &Deployment) -> LinuxPidFd {
    let pid_file = deployment.endpoint.with_file_name("fake-node.pid");
    let mut handle = None;
    until(|| {
        handle = fs::read_to_string(&pid_file)
            .ok()
            .and_then(|pid| pid.trim().parse::<u32>().ok())
            .and_then(|pid| linux_process(pid).ok())
            .and_then(|stat| LinuxPidFd::from_observation(&stat).ok());
        handle.is_some()
    });
    handle.unwrap()
}

/// Polls externally visible evidence under a failure deadline rather than sleeping a fixed time.
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 20);
    while !ready() {
        assert!(Instant::now() < deadline, "observation timed out");
        std::thread::sleep(Duration::from_millis(/*millis*/ 10));
    }
}

/// The TCP address the executable reported as bound, read from its structured log line.
fn listening(log: &str) -> Option<String> {
    log.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["message"] == "ora-controller listening")
        .and_then(|event| {
            event["context"]["endpoint"]
                .as_str()
                .and_then(|endpoint| endpoint.strip_prefix("tcp://"))
                .map(str::to_owned)
        })
}

/// Byte offset of a log line so ordering assertions read the actual sequence of events.
fn position(log: &str, needle: &str) -> usize {
    log.find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?} in log:\n{log}"))
}

/// Refused flags and configuration exit before any Controller state or Node exists.
#[test]
fn refused_composition_leaves_no_state_behind() {
    let deployment = deployment(FakeNode::Listen, 5000, 5000);
    let config = deployment.config.to_str().unwrap();
    let relative = Path::new("controller.json");
    let refusals: Vec<Vec<&str>> = vec![
        vec!["--config", config, "--transport", "unix", "--port", "1"],
        vec!["--config", config, "--transport", "unix"],
        vec!["--config", config, "--socket", "/tmp/api.sock"],
        vec!["--config", relative.to_str().unwrap()],
        vec![
            "--config",
            config,
            "--transport",
            "unix",
            "--socket",
            "relative.sock",
        ],
    ];
    for args in refusals {
        let mut executable = Executable::start(deployment.root.path(), &args);
        assert!(!executable.wait().success(), "{args:?}");
        assert!(
            !deployment.home.exists(),
            "{args:?} created Controller state"
        );
    }
    // A hosting request whose Node configuration binds another Controller is refused read-only.
    let original = fs::read(&deployment.node_config).unwrap();
    let foreign: serde_json::Value = serde_json::json!({ "control": { "controller_id": "other", "listen": { "kind": "ipc", "path": deployment.endpoint } } });
    fs::write(
        &deployment.node_config,
        serde_json::to_vec(&foreign).unwrap(),
    )
    .unwrap();
    let mut executable = Executable::start(
        deployment.root.path(),
        &["--config", config, "--single-node", "--port", "0"],
    );
    assert!(!executable.wait().success());
    assert!(
        executable
            .errors()
            .contains("does not bind this Controller")
    );
    assert!(!deployment.home.exists());
    assert_eq!(
        fs::read(&deployment.node_config).unwrap(),
        serde_json::to_vec(&foreign).unwrap()
    );
    assert!(!deployment.endpoint.with_file_name("fake-node.pid").exists());
    fs::write(&deployment.node_config, original).unwrap();
}

/// A non-loopback listener is allowed but leaves an explicit warning in the log.
#[test]
fn non_loopback_listener_warns() {
    let deployment = deployment(FakeNode::Listen, 5000, 5000);
    let mut executable = Executable::start(
        deployment.root.path(),
        &[
            "--config",
            deployment.config.to_str().unwrap(),
            "--host",
            "0.0.0.0",
            "--port",
            "0",
        ],
    );
    let address = executable.address();
    assert!(address.starts_with("0.0.0.0:"));
    assert!(
        executable
            .log()
            .contains("non-loopback address without authentication")
    );
    assert!(executable.terminate().success());
}

/// Readiness failure terminates the stand-in, never prints an address and releases the lease.
#[test]
fn readiness_timeout_stops_node_and_never_opens_api() {
    let deployment = deployment(FakeNode::Silent, 300, 5000);
    let mut executable = Executable::start(
        deployment.root.path(),
        &[
            "--config",
            deployment.config.to_str().unwrap(),
            "--single-node",
            "--port",
            "0",
        ],
    );
    let node = fake_node(&deployment);
    let status = executable.wait();
    assert!(!status.success());
    assert!(executable.errors().contains("did not become ready"));
    assert!(!executable.log().contains("ora-controller listening"));
    until(|| node.has_exited().unwrap());
    // The lease is free again: the same identity reopens the state the refused run created.
    drop(SqliteStore::open(&deployment.home, ControllerId::new("owner")).unwrap());
}

/// Normal stop follows the fixed order and retires the hosted Node before the process exits.
#[test]
fn normal_stop_orders_phases_and_retires_node() {
    let deployment = deployment(FakeNode::Listen, 5000, 5000);
    let node_config = fs::read(&deployment.node_config).unwrap();
    let mut executable = Executable::start(
        deployment.root.path(),
        &[
            "--config",
            deployment.config.to_str().unwrap(),
            "--single-node",
            "--port",
            "0",
        ],
    );
    let address = executable.address();
    let node = fake_node(&deployment);
    let log = executable.log();
    assert!(position(&log, "managed Node ready") < position(&log, "ora-controller listening"));
    assert!(std::net::TcpStream::connect(&address).is_ok());
    assert!(executable.terminate().success());
    let log = executable.log();
    let api = position(&log, "API admission stopped");
    let sessions = position(&log, "Node sessions stopped");
    let stopped = position(&log, "managed Node stopped");
    assert!(api < sessions && sessions < stopped, "{log}");
    assert!(node.has_exited().unwrap());
    assert!(std::net::TcpStream::connect(&address).is_err());
    assert_eq!(fs::read(&deployment.node_config).unwrap(), node_config);
}

/// A Node that ignores SIGTERM is left to host/guardian containment after a warning, never killed.
#[test]
fn stop_timeout_warns_without_sigkill() {
    let deployment = deployment(FakeNode::Stubborn, 5000, 300);
    let mut executable = Executable::start(
        deployment.root.path(),
        &[
            "--config",
            deployment.config.to_str().unwrap(),
            "--single-node",
            "--port",
            "0",
        ],
    );
    executable.address();
    let node = fake_node(&deployment);
    let started = Instant::now();
    assert!(executable.terminate().success());
    assert!(started.elapsed() < Duration::from_secs(/*secs*/ 10));
    assert!(
        executable
            .log()
            .contains("did not stop within stop_timeout_ms")
    );
    assert!(!node.has_exited().unwrap());
    node.signal(ProcessSignal::Kill).unwrap();
    until(|| node.has_exited().unwrap());
}

/// Losing the hosted Node stops admission and exits with failure while accepted records survive.
#[test]
fn node_exit_stops_admission_and_keeps_records() {
    let deployment = deployment(FakeNode::Listen, 5000, 5000);
    let mut executable = Executable::start(
        deployment.root.path(),
        &[
            "--config",
            deployment.config.to_str().unwrap(),
            "--single-node",
            "--port",
            "0",
        ],
    );
    let address = executable.address();
    let node = fake_node(&deployment);
    let accepted: MiniCloneAccepted = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            reqwest::Client::new()
                .post(format!("http://{address}/api/clones"))
                .json(&MiniCloneRequest {
                    request_id: "before-node-loss".into(),
                    repository: "https://example.com/repo.git".into(),
                    branch: "main".into(),
                })
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap()
        });
    node.signal(ProcessSignal::Kill).unwrap();
    let status = executable.wait();
    assert!(!status.success());
    assert!(
        executable
            .errors()
            .contains("managed Node exited unexpectedly")
    );
    assert!(std::net::TcpStream::connect(&address).is_err());
    let owner = SqliteStore::open(&deployment.home, ControllerId::new("owner")).unwrap();
    let operations = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(owner.operations())
        .unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(
        operations[0].command.execution_id.as_str(),
        accepted.execution_id
    );
    assert_eq!(
        operations[0]
            .command
            .request_id
            .as_ref()
            .map(RequestId::as_str),
        Some("before-node-loss")
    );
    assert!(operations[0].result.is_none());
}

/// A cloud deployment binds no JSON surface, refuses listener flags and creates no local state;
/// with Cloud unreachable it stays up, ineligible, until asked to stop.
#[test]
fn cloud_persistence_serves_no_surface_and_creates_no_local_state() {
    let root = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
        .tempdir_in(std::env::var_os("HOME").unwrap())
        .unwrap();
    let path = root.path();
    let home = path.join("controller");
    let config = path.join("controller.json");
    fs::write(
        &config,
        serde_json::to_vec(&serde_json::json!({
            "controller": {
                "home_directory": home,
                "persistence": {
                    "kind": "cloud",
                    "endpoint": "http://127.0.0.1:1",
                    "claim_interval_ms": 100,
                },
                "protected_state_directories": [path.join("node")],
                "controller_id": "owner",
                "nodes": [{ "node_id": "node", "endpoint": { "kind": "ipc", "path": path.join("node").join("control.sock") } }],
                "session": { "io_timeout_ms": 500, "query_interval_ms": 100 },
                "reconnect_ms": 200,
                "timezone": "Asia/Shanghai",
            },
        }))
        .unwrap(),
    )
    .unwrap();
    let config = config.to_str().unwrap();
    let mut refused = Executable::start(path, &["--config", config, "--port", "0"]);
    assert!(!refused.wait().success());
    assert!(refused.errors().contains("serves no JSON surface"));
    assert!(!home.exists());
    let mut executable = Executable::start(path, &["--config", config]);
    until(|| {
        let log = executable.log();
        log.contains("ora-controller coordinating through cloud")
            && log.contains("http://127.0.0.1:1")
    });
    until(|| executable.log().contains("Cloud lease not acquired"));
    assert!(!executable.log().contains("ora-controller listening"));
    assert!(!home.exists());
    assert!(executable.terminate().success());
    assert!(!home.exists());
}
