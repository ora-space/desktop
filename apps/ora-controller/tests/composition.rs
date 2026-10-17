#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Executable-level composition tests. A child-only stand-in replaces `ora-node` at the process
//! boundary so readiness, stop and crash behavior can be shaped without Git, host or guardian.
#[path = "support/local_clone.rs"]
mod local_clone;
use ora_controller::{CoordinationStore, SqliteStore};
use ora_node_protocol::{ControllerId, NodeId};
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
    /// Waits until the composition is up; the executable logs it only after its Node is ready.
    fn ready(&self) {
        until(|| self.log().contains(COMPOSED));
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

/// The structured event a local composition logs once its owner and hosted Node are up.
const COMPOSED: &str = "ora-controller coordinating locally";

/// Byte offset of a log line so ordering assertions read the actual sequence of events.
fn position(log: &str, needle: &str) -> usize {
    log.find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?} in log:\n{log}"))
}

/// Refused flags and configuration exit before any Controller state or Node exists; listener
/// flags are no longer part of the executable at all.
#[test]
fn refused_composition_leaves_no_state_behind() {
    let deployment = deployment(FakeNode::Listen, 5000, 5000);
    let config = deployment.config.to_str().unwrap();
    let relative = Path::new("controller.json");
    let refusals: Vec<Vec<&str>> = vec![
        vec!["--config", config, "--port", "0"],
        vec!["--config", config, "--transport", "tcp"],
        vec!["--config", config, "--socket", "/tmp/api.sock"],
        vec!["--config", relative.to_str().unwrap()],
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
        &["--config", config, "--single-node"],
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

/// Readiness failure terminates the stand-in, never reports a composition and releases the lease.
#[test]
fn readiness_timeout_stops_node_and_never_composes() {
    let deployment = deployment(FakeNode::Silent, 300, 5000);
    let mut executable = Executable::start(
        deployment.root.path(),
        &[
            "--config",
            deployment.config.to_str().unwrap(),
            "--single-node",
        ],
    );
    let node = fake_node(&deployment);
    let status = executable.wait();
    assert!(!status.success());
    assert!(executable.errors().contains("did not become ready"));
    assert!(!executable.log().contains(COMPOSED));
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
        ],
    );
    executable.ready();
    let node = fake_node(&deployment);
    let log = executable.log();
    assert!(position(&log, "managed Node ready") < position(&log, COMPOSED));
    assert!(executable.terminate().success());
    let log = executable.log();
    let sessions = position(&log, "Node sessions stopped");
    let stopped = position(&log, "managed Node stopped");
    assert!(sessions < stopped, "{log}");
    assert!(node.has_exited().unwrap());
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
        ],
    );
    executable.ready();
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

/// Losing the hosted Node stops the composition and exits with failure while accepted records survive.
#[test]
fn node_exit_stops_composition_and_keeps_records() {
    let deployment = deployment(FakeNode::Listen, 5000, 5000);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // A dispatch the database already holds; the executable never accepts work itself.
    let owner = SqliteStore::open(&deployment.home, ControllerId::new("owner")).unwrap();
    let accepted = local_clone::seed(&deployment.home, "before-node-loss");
    drop(owner);
    let mut executable = Executable::start(
        deployment.root.path(),
        &[
            "--config",
            deployment.config.to_str().unwrap(),
            "--single-node",
        ],
    );
    executable.ready();
    let node = fake_node(&deployment);
    node.signal(ProcessSignal::Kill).unwrap();
    let status = executable.wait();
    assert!(!status.success());
    assert!(
        executable
            .errors()
            .contains("managed Node exited unexpectedly")
    );
    let owner = SqliteStore::open(&deployment.home, ControllerId::new("owner")).unwrap();
    assert_eq!(
        runtime
            .block_on(owner.pending_dispatches(&NodeId::new("node")))
            .unwrap(),
        vec![accepted]
    );
}

/// A cloud deployment creates no local state; with Cloud unreachable it stays up, ineligible,
/// until asked to stop.
#[test]
fn cloud_persistence_serves_no_surface_and_creates_no_local_state() {
    let root = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
        .tempdir_in(std::env::var_os("HOME").unwrap())
        .unwrap();
    let path = root.path();
    let home = path.join("controller");
    let config = path.join("controller.json");
    let key = rcgen::KeyPair::generate().unwrap();
    let mut ca = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
    let ca = rcgen::CertifiedIssuer::self_signed(ca, key).unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let mut parameters = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    parameters.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
    let cert = parameters.signed_by(&key, &ca).unwrap();
    fs::write(path.join("controller.pem"), cert.pem()).unwrap();
    fs::write(path.join("controller.key"), key.serialize_pem()).unwrap();
    fs::write(path.join("ca.pem"), ca.pem()).unwrap();
    fs::write(
        &config,
        serde_json::to_vec(&serde_json::json!({
            "controller": {
                "home_directory": home,
                "persistence": {
                    "kind": "cloud",
                    "endpoint": "https://127.0.0.1:1",
                    "claim_interval_ms": 100,
                },
                "management_tls": {"certificate_file":path.join("controller.pem"),"private_key_file":path.join("controller.key"),"ca_file":path.join("ca.pem")},
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
    let mut executable = Executable::start(path, &["--config", config]);
    until(|| {
        let log = executable.log();
        log.contains("ora-controller coordinating through cloud")
            && log.contains("https://127.0.0.1:1")
    });
    until(|| executable.log().contains("Cloud lease not acquired"));
    assert!(!home.exists());
    assert!(executable.terminate().success());
    assert!(!home.exists());
}
