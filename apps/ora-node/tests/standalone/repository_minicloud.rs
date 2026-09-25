use super::*;
use crate::support::{ChildGuard, until};
use ora_contracts::controller_api::*;
use ora_controller::{
    ApiConfig, DeploymentConfig, NodeEndpoint, NodeHosting, NodeTarget, Persistence, RuntimeConfig,
    SessionConfig,
};
use pretty_assertions::assert_eq;
use std::{
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::Duration,
};

enum Entry {
    Http,
    Vite,
}

/// Drops a real HTTP response body only after observing the upstream durable acceptance receipt.
async fn truncated_acceptance(address: String, input: &MiniCloneRequest) -> MiniCloneAccepted {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/api/clones", listener.local_addr().unwrap());
    let relay = tokio::spawn(async move {
        let (mut downstream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(downstream.read_u8().await.unwrap());
            assert!(request.len() < 8192);
        }
        let length: usize = String::from_utf8_lossy(&request)
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        let mut body = vec![0; length];
        downstream.read_exact(&mut body).await.unwrap();
        request.extend(body);
        let mut upstream = tokio::net::TcpStream::connect(address).await.unwrap();
        upstream.write_all(&request).await.unwrap();
        let mut response = Vec::new();
        upstream.read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 202"));
        let start = response
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap()
            + 4;
        let receipt = serde_json::from_slice::<MiniCloneAccepted>(&response[start..]).unwrap();
        downstream.write_all(&response[..start + 1]).await.unwrap();
        downstream.shutdown().await.unwrap();
        receipt
    });
    let response = reqwest::Client::new()
        .post(endpoint)
        .header("connection", "close")
        .json(input)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 202);
    assert!(response.json::<MiniCloneAccepted>().await.is_err());
    relay.await.unwrap()
}

/// Starts the production Controller executable on loopback and reads its actual bound address.
/// Port 0 picks an ephemeral port; restarts pass the first address's port back to keep clients stable.
pub(super) fn launch(
    fixture: &Fixture,
    config: &DeploymentConfig,
    port: u16,
    hosting: NodeHosting,
) -> (ChildGuard, String) {
    let path = fixture.path().join("controller.json");
    fs::write(&path, serde_json::to_vec(config).unwrap()).unwrap();
    let log = fixture.path().join("controller.log");
    let mut command = Command::new(
        std::path::Path::new(env!("CARGO_BIN_EXE_ora-node")).with_file_name("ora-controller"),
    );
    command
        .arg("--config")
        .arg(path)
        .args(["--transport", "tcp", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string());
    match hosting {
        NodeHosting::Managed => {
            command.arg("--single-node");
        }
        NodeHosting::External => {}
    }
    // Lead a fresh process group like the launcher's setsid does, so hosting tests can address the
    // Controller and its Node together without signaling the test runner's own group.
    let child = ChildGuard(
        command
            .process_group(/*pgroup*/ 0)
            .stdin(Stdio::null())
            .stdout(fs::File::create(&log).unwrap())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("build ora-controller before standalone acceptance"),
    );
    let mut address = None;
    until(|| {
        // The bound endpoint is a structured log event, ordered with the rest of the log.
        address = fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|event| event["message"] == "ora-controller listening")
            .and_then(|event| {
                event["context"]["endpoint"]
                    .as_str()
                    .and_then(|endpoint| endpoint.strip_prefix("tcp://"))
                    .map(str::to_owned)
            });
        address.is_some()
    });
    (child, address.unwrap())
}

/// Extracts the bound port so a restarted executable reuses the address clients already hold.
fn port_of(address: &str) -> u16 {
    address.rsplit(':').next().unwrap().parse().unwrap()
}

/// Exercises actual proxy/HTTP, independent Controller death, Node and HTTPS Git without a fake coordinator.
fn exercise(entry: Entry) {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let expected_commit = fixture.git(&["rev-parse", "main"]);
        let source = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        source.paused.store(true, Ordering::SeqCst);
        let clone = configuration(&fixture, &source);
        let mut node = ipc::launch(&fixture, &clone);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        let config = DeploymentConfig {
            api: Some(ApiConfig {
                node_id: NodeId::new("test-node"),
            }),
            single_node: None,
            controller: RuntimeConfig {
                home_directory: fixture.path().join("controller"),
                persistence: Persistence::Sqlite,
                protected_state_directories: vec![
                    fixture.config().home_directory,
                    fixture.process().host_directory,
                ],
                controller_id: ControllerId::new("owner"),
                nodes: vec![NodeTarget {
                    node_id: NodeId::new("test-node"),
                    endpoint: NodeEndpoint::Ipc {
                        path: fixture.config().home_directory.join("control.sock"),
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
        let (mut server, address) =
            launch(&fixture, &config, /*port*/ 0, NodeHosting::External);
        let port = port_of(&address);
        let mut vite = None;
        let base = match entry {
            Entry::Http => format!("http://{address}"),
            Entry::Vite => {
                let log = fixture.path().join("vite.log");
                // Vite treats port 0 as unset and falls back to its default, which another
                // development server on the host may hold; reserve a free loopback port instead.
                let vite_port = std::net::TcpListener::bind("127.0.0.1:0")
                    .unwrap()
                    .local_addr()
                    .unwrap()
                    .port();
                vite = Some(ChildGuard(
                    Command::new(
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("../../node_modules/.bin/vite"),
                    )
                    .current_dir(
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("../minicloud/client"),
                    )
                    .args(["--port", &vite_port.to_string()])
                    .env("MINICLOUD_SERVER_URL", format!("http://{address}"))
                    .env("NO_COLOR", "1")
                    .stdout(fs::File::create(&log).unwrap())
                    .stderr(Stdio::inherit())
                    .stdin(Stdio::null())
                    .spawn()
                    .unwrap(),
                ));
                let mut url = None;
                until(|| {
                    url = fs::read_to_string(&log)
                        .unwrap_or_default()
                        .split_whitespace()
                        .find(|part| part.starts_with("http://127.0.0.1:"))
                        .map(|part| part.trim_end_matches('/').to_owned());
                    url.is_some()
                });
                url.unwrap()
            }
        };
        let execution = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let client = reqwest::Client::builder()
                    .pool_max_idle_per_host(/*max*/ 0)
                    .build()
                    .unwrap();
                if vite.is_some() {
                    let page = client
                        .get(&base)
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap();
                    assert!(page.contains("/src/main.tsx"));
                }
                let endpoint = format!("{base}/api/clones");
                let input = MiniCloneRequest {
                    request_id: "web-request".into(),
                    repository: source.address.clone(),
                    branch: "main".into(),
                };
                // Hold a real SQLite writer lock at the persistence seam. HTTP must not report
                // acceptance or leave dispatchable intent when its durable write cannot commit.
                let locked = rusqlite::Connection::open(
                    config
                        .controller
                        .home_directory
                        .join("ora-controller.sqlite3"),
                )
                .unwrap();
                locked.execute_batch("BEGIN IMMEDIATE").unwrap();
                let rejected = client.post(&endpoint).json(&input).send().await.unwrap();
                assert_eq!(rejected.status().as_u16(), 503);
                locked.execute_batch("ROLLBACK").unwrap();
                drop(locked);
                assert_eq!(
                    client
                        .get(&endpoint)
                        .send()
                        .await
                        .unwrap()
                        .json::<Vec<MiniCloneOperation>>()
                        .await
                        .unwrap(),
                    vec![]
                );
                let receipt = tokio::time::timeout(
                    Duration::from_secs(/*secs*/ 10),
                    truncated_acceptance(address.clone(), &input),
                )
                .await
                .unwrap();
                server.kill();
                let (replacement, _) = launch(&fixture, &config, port, NodeHosting::External);
                server = replacement;
                assert_eq!(
                    client
                        .post(&endpoint)
                        .json(&input)
                        .send()
                        .await
                        .unwrap()
                        .json::<MiniCloneAccepted>()
                        .await
                        .unwrap(),
                    receipt
                );
                // A .git directory is Git's observable side effect, not just Controller acceptance.
                // Keep HTTPS paused while normally stopping the coordinator during the live clone.
                until(|| {
                    fs::read_dir(&clone.repository_root)
                        .unwrap()
                        .flatten()
                        .any(|entry| entry.path().join(".git").is_dir())
                });
                let mut git = None;
                until(|| {
                    git = ora_utils::process::linux_process_snapshot()
                        .unwrap()
                        .flatten()
                        .find_map(|stat| {
                            let executable = std::path::Path::new("/proc")
                                .join(stat.pid.to_string())
                                .join("exe");
                            if fs::read_link(executable).ok().as_ref()
                                == Some(&fixture.path().join("git"))
                            {
                                ora_utils::process::LinuxPidFd::from_observation(&stat).ok()
                            } else {
                                None
                            }
                        });
                    git.is_some()
                });
                let git = git.unwrap();
                server.terminate();
                assert!(server.0.try_wait().unwrap().unwrap().success());
                assert!(node.0.try_wait().unwrap().is_none());
                assert!(!git.has_exited().unwrap());
                source.paused.store(false, Ordering::SeqCst);
                let (replacement, _) = launch(&fixture, &config, port, NodeHosting::External);
                server = replacement;
                let final_record = tokio::time::timeout(Duration::from_secs(/*secs*/ 40), async {
                    loop {
                        let records: Vec<MiniCloneOperation> = client
                            .get(&endpoint)
                            .send()
                            .await
                            .unwrap()
                            .json()
                            .await
                            .unwrap();
                        assert_eq!(records.len(), 1);
                        if !matches!(records[0].state, MiniCloneState::Pending) {
                            break records[0].clone();
                        }
                        tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                    }
                })
                .await
                .unwrap();
                let MiniCloneState::Succeeded {
                    ref path,
                    ref commit,
                } = final_record.state
                else {
                    panic!("{final_record:?}");
                };
                assert_eq!(commit, &expected_commit);
                assert!(std::path::Path::new(path).join(".git").is_dir());
                source.reject_auth.store(true, Ordering::SeqCst);
                server.kill();
                let (replacement, _) = launch(&fixture, &config, port, NodeHosting::External);
                server = replacement;
                let restored: MiniCloneOperation = client
                    .get(format!("{endpoint}/{}", receipt.execution_id))
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                assert_eq!(restored, final_record);
                assert_eq!(receipt.execution_id, restored.execution_id);
                ExecutionId::new(receipt.execution_id)
            });
        // Vite handles normal termination; reap it before deleting its fixture output directory.
        if let Some(mut vite) = vite {
            vite.terminate();
        }
        server.terminate();
        node.terminate();
        let database = ora_node_db::NodeDatabase::open(
            &fixture.config().home_directory.join("ora-node.sqlite3"),
            fixture.config().identity,
        )
        .unwrap();
        assert_eq!(
            database
                .process_journal()
                .unwrap()
                .attempts(&execution)
                .unwrap()
                .len(),
            1
        );
    });
}

/// Ordinary crates acceptance needs no JavaScript installation and exercises the independent executable.
#[test]
fn minicloud_http_clone_survives_controller_kill_and_replays_original_intent() {
    exercise(Entry::Http);
}

/// Explicit development acceptance adds real Vite proxy; run after installing frontend dependencies.
#[test]
#[ignore = "requires deno install; run the minicloud_vite filter explicitly with --ignored"]
fn minicloud_vite_proxy_reaches_real_https_clone_and_restart() {
    exercise(Entry::Vite);
}
