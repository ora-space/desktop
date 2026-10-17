use super::*;
use crate::support::{ChildGuard, block_on, until};
use ora_controller::{
    DeploymentConfig, NodeEndpoint, NodeTarget, Persistence, RuntimeConfig, SessionConfig,
    SingleNodeConfig, SqliteStore,
};
use ora_utils::process::{LinuxPidFd, ProcessSignal, linux_process_snapshot};
use pretty_assertions::assert_eq;
use std::{
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::Duration,
};

/// Pins the Node the Controller started; test failure must not leave it listening in a deleted root.
struct HostedNode(LinuxPidFd, u32);
impl Drop for HostedNode {
    fn drop(&mut self) {
        let _ = self.0.signal(ProcessSignal::Kill);
    }
}

/// Finds the one Node executable started from this fixture's IPC configuration, never by remembered PID.
fn hosted_node(config: &std::path::Path) -> HostedNode {
    let mut handle = None;
    until(|| {
        handle = linux_process_snapshot()
            .unwrap()
            .flatten()
            .find_map(|stat| {
                let proc = std::path::Path::new("/proc").join(stat.pid.to_string());
                let cmdline = fs::read(proc.join("cmdline")).unwrap_or_default();
                (fs::read_link(proc.join("exe")).ok().as_deref()
                    == Some(std::path::Path::new(env!("CARGO_BIN_EXE_ora-node")))
                    && cmdline
                        .split(|byte| *byte == 0)
                        .any(|arg| arg == config.as_os_str().as_encoded_bytes()))
                .then(|| {
                    LinuxPidFd::from_observation(&stat)
                        .ok()
                        .map(|handle| (handle, stat.pid))
                })
                .flatten()
            });
        handle.is_some()
    });
    let (handle, pid) = handle.unwrap();
    HostedNode(handle, pid)
}

/// Reads the process group from procfs; `LinuxProcessStat` deliberately exposes only the session.
fn process_group(pid: u32) -> u32 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    stat[stat.rfind(')').unwrap() + 2..]
        .split(' ')
        .nth(2)
        .unwrap()
        .parse()
        .unwrap()
}

/// Pins the fixture's live Git process so cleanup can be asserted independently of Node's view.
fn live_git(fixture: &Fixture) -> LinuxPidFd {
    let mut git = None;
    until(|| {
        git = linux_process_snapshot()
            .unwrap()
            .flatten()
            .find_map(|stat| {
                let executable = std::path::Path::new("/proc")
                    .join(stat.pid.to_string())
                    .join("exe");
                (fs::read_link(executable).ok().as_ref() == Some(&fixture.path().join("git")))
                    .then(|| LinuxPidFd::from_observation(&stat).ok())
                    .flatten()
            });
        git.is_some()
    });
    git.unwrap()
}

/// The composed executable hosts its Node: Controller death leaves Node and Git running, a live
/// endpoint refuses a second hosting Controller, and normal stop retires sessions and Node in order.
#[test]
fn single_node_composition_hosts_and_retires_its_node() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let expected_commit = fixture.git(&["rev-parse", "main"]);
        let source =
            GitServerFixture::new(fixture.path(), fixture.path().join("main").join(".git"));
        source.pause(true);
        let clone = configuration(&fixture, &source);
        let node_config = ipc::write_config(&fixture, &clone, /*frame_timeout_ms*/ 40_000);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let home = fixture.path().join("controller");
        let config = DeploymentConfig {
            single_node: Some(SingleNodeConfig {
                node_executable: env!("CARGO_BIN_EXE_ora-node").into(),
                node_config: node_config.clone(),
                ready_timeout_ms: 15_000,
                stop_timeout_ms: 15_000,
            }),
            controller: RuntimeConfig {
                management_tls: None,
                home_directory: home.clone(),
                persistence: Persistence::Sqlite,
                protected_state_directories: vec![
                    fixture.config().home_directory,
                    fixture.process().host_directory,
                ],
                controller_id: ControllerId::new("owner"),
                nodes: vec![NodeTarget {
                    node_id: NodeId::new("test-node"),
                    endpoint: NodeEndpoint::Ipc {
                        path: endpoint.clone(),
                    },
                }],
                session: SessionConfig {
                    io_timeout_ms: 5000,
                    query_interval_ms: 100,
                },
                reconnect_ms: 100,
                timezone: "Asia/Shanghai".into(),
            },
        };
        // A hosting Controller refuses to start when Node configuration binds another owner.
        let mut foreign: serde_json::Value =
            serde_json::from_slice(&fs::read(&node_config).unwrap()).unwrap();
        foreign["control"]["controller_id"] = "someone-else".into();
        let foreign_path = fixture.path().join("foreign-node.json");
        fs::write(&foreign_path, serde_json::to_vec(&foreign).unwrap()).unwrap();
        let mut misbound = config.clone();
        misbound.single_node.as_mut().unwrap().node_config = foreign_path;
        let (mut rejected, _) = launch_expecting_exit(&fixture, &misbound);
        assert!(!rejected.0.wait().unwrap().success());
        assert!(!home.exists());

        // The executable has no intake of its own: work is accepted durably by the same owner
        // before the composition starts, and the Controller dispatches it as pending work.
        let hosted = accept(&home, &source, "hosted");
        let node_config_bytes = fs::read(&node_config).unwrap();
        let mut controller = launch(&fixture, &config);
        let first = hosted_node(&node_config);
        // The hosted Node shares the Controller's process group without a new session of its own.
        let group = controller.0.id();
        assert_eq!(process_group(first.1), group);
        // The Node's snapshot fetch now waits on the paused Git Server.
        until(|| {
            fs::read_dir(clone.repository_root.join(".ora-transport"))
                .is_ok_and(|mut entries| entries.next().is_some())
        });
        // Killing only the Controller must not signal the Node it started or the Git it accepted.
        controller.kill();
        assert!(!first.0.has_exited().unwrap());
        let (mut refused, _) = launch_expecting_exit(&fixture, &config);
        assert!(!refused.0.wait().unwrap().success());
        assert!(!first.0.has_exited().unwrap());
        source.pause(false);
        // The surviving Node finishes the clone on its own and replays the unacknowledged result
        // to whoever holds the owner session; observing it without an Ack leaves the event for the
        // replacement Controller to take over durably.
        // The timer must be created inside the runtime that drives it.
        block_on(async {
            tokio::time::timeout(Duration::from_secs(/*secs*/ 40), async {
                let hello = ControllerToNodeMessage::Hello(HelloMessage {
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    payload: Hello {
                        controller_id: ControllerId::new("owner"),
                        supported_versions: vec![CURRENT_PROTOCOL_VERSION],
                    },
                });
                loop {
                    // The dead Controller's session is revoked asynchronously; until then the Node
                    // closes extra connections, which surfaces as a failed write or EOF.
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                    let Ok(mut stream) = tokio::net::UnixStream::connect(&endpoint).await else {
                        continue;
                    };
                    if write_controller_message(&mut stream, &hello).await.is_err() {
                        continue;
                    }
                    let Ok(Some(NodeToControllerMessage::HelloAccepted(_))) =
                        read_node_message(&mut stream).await
                    else {
                        continue;
                    };
                    while let Ok(Some(message)) = read_node_message(&mut stream).await {
                        if let NodeToControllerMessage::CloneResult(result) = message
                            && result.execution_id == hosted.execution_id
                        {
                            return;
                        }
                    }
                }
            })
            .await
        })
        .unwrap();
        // With the Controller gone, a group-level stop still reaches the Node it started.
        // SAFETY: signals only the process group this test created and observed.
        assert_eq!(
            unsafe { libc::kill(-(group as libc::pid_t), libc::SIGTERM) },
            0
        );
        until(|| first.0.has_exited().unwrap());
        // A replacement composition starts a fresh Node, which replays the original result; the
        // Node retires it from its outbox only after the Controller committed and acknowledged it.
        controller = launch(&fixture, &config);
        let second = hosted_node(&node_config);
        until(|| outbox_len(&fixture) == 0);
        // Normal stop retires the hosted Node before the process exits.
        controller.terminate();
        assert!(controller.0.try_wait().unwrap().unwrap().success());
        assert!(second.0.has_exited().unwrap());
        let succeeded = operations(&home);
        let [ref record] = succeeded[..] else {
            panic!("{succeeded:?}");
        };
        assert_eq!(record.command, hosted);
        let Some(CloneExecutionResult::CloneReady(ready)) = &record.result else {
            panic!("{record:?}");
        };
        assert_eq!(ready.commit, CommitId::new(&expected_commit));

        // Normal stop during a live clone: the hosted Node finishes its managed-Git cleanup and
        // exits before the Controller does; the accepted record is neither lost nor turned into a
        // fresh execution when the composition comes back.
        source.pause(true);
        let interrupted = accept(&home, &source, "hosted-interrupted");
        controller = launch(&fixture, &config);
        let third = hosted_node(&node_config);
        let git = live_git(&fixture);
        controller.terminate();
        assert!(controller.0.try_wait().unwrap().unwrap().success());
        assert!(third.0.has_exited().unwrap());
        assert!(git.has_exited().unwrap());
        source.pause(false);
        controller = launch(&fixture, &config);
        let fourth = hosted_node(&node_config);
        controller.terminate();
        assert!(controller.0.try_wait().unwrap().unwrap().success());
        assert!(fourth.0.has_exited().unwrap());
        let mut records = operations(&home);
        records.sort_by(|a, b| a.command.execution_id.cmp(&b.command.execution_id));
        let mut expected = vec![hosted.clone(), interrupted];
        expected.sort_by(|a, b| a.execution_id.cmp(&b.execution_id));
        assert_eq!(
            records
                .iter()
                .map(|record| record.command.clone())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(records.contains(record));
        assert_eq!(fs::read(&node_config).unwrap(), node_config_bytes);
        let database = ora_node_db::NodeDatabase::open(
            &fixture.config().home_directory.join("ora-node.sqlite3"),
            fixture.config().identity,
        )
        .unwrap();
        assert_eq!(
            database
                .process_journal()
                .unwrap()
                .attempts(&hosted.execution_id)
                .unwrap()
                .len(),
            // One Run per workload step of the clone.
            3
        );
    });
}

/// Accepts one clone through the durable local owner while no Controller process holds it.
fn accept(
    home: &std::path::Path,
    source: &GitServerFixture,
    request_id: &str,
) -> CloneRepositoryMessage {
    let owner = SqliteStore::open(home, ControllerId::new("owner")).unwrap();
    let command = seed_dispatch(
        home,
        request(source, request_id, "main"),
        RequestId::new(request_id),
    );
    drop(owner);
    command
}

/// Reads every accepted operation once the Controller process has released the owner.
fn operations(home: &std::path::Path) -> Vec<HistoricalClone> {
    historical_clones(home)
}

/// Counts the Node's unacknowledged result events.
fn outbox_len(fixture: &Fixture) -> i64 {
    rusqlite::Connection::open(fixture.config().home_directory.join("ora-node.sqlite3"))
        .unwrap()
        .query_row("SELECT count(*) FROM clone_outbox", [], |row| {
            row.get(/*idx*/ 0)
        })
        .unwrap()
}

/// Starts a hosting Controller that is expected to refuse composition; callers reap its exit status.
fn launch_expecting_exit(
    fixture: &Fixture,
    config: &DeploymentConfig,
) -> (ChildGuard, std::path::PathBuf) {
    let path = fixture.path().join("refused-controller.json");
    fs::write(&path, serde_json::to_vec(config).unwrap()).unwrap();
    let child = ChildGuard(
        std::process::Command::new(
            std::path::Path::new(env!("CARGO_BIN_EXE_ora-node")).with_file_name("ora-controller"),
        )
        .arg("--config")
        .arg(&path)
        .arg("--single-node")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .unwrap(),
    );
    (child, path)
}

/// Starts the production hosting Controller executable and waits until it has composed.
fn launch(fixture: &Fixture, config: &DeploymentConfig) -> ChildGuard {
    let path = fixture.path().join("controller.json");
    fs::write(&path, serde_json::to_vec(config).unwrap()).unwrap();
    let log = fixture.path().join("controller.log");
    // Lead a fresh process group so the test can address the Controller and its Node together
    // without signaling the test runner's own group.
    let child = ChildGuard(
        Command::new(
            std::path::Path::new(env!("CARGO_BIN_EXE_ora-node")).with_file_name("ora-controller"),
        )
        .arg("--config")
        .arg(path)
        .arg("--single-node")
        .process_group(/*pgroup*/ 0)
        .stdin(Stdio::null())
        .stdout(fs::File::create(&log).unwrap())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("build ora-controller before standalone acceptance"),
    );
    // Composition is complete once this structured event is in the log, after the Node is ready.
    until(|| {
        fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|event| event["message"] == "ora-controller coordinating locally")
    });
    child
}
