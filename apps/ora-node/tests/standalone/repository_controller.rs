use super::*;
use crate::support::{ChildGuard, block_on, until};
use ora_controller::{CloneIntake, CoordinationStore, ExecutionOutcome, SqliteStore};
use pretty_assertions::assert_eq;
use std::{
    process::{Command, Stdio},
    sync::{Arc, atomic::AtomicBool, mpsc},
    time::Duration,
};
use tokio::net::{UnixListener, UnixStream};

/// A byte-stream fault boundary drops acknowledgements without replacing either production peer.
pub(super) struct Proxy {
    pub(super) endpoint: PathBuf,
    pub(super) drop_ack: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    pub(super) acknowledgements: mpsc::Receiver<EventAckMessage>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Proxy {
    /// Listens in a distinct private deployment root and forwards the actual Node framing in both directions.
    pub(super) fn new(fixture: &Fixture) -> Self {
        let root = fixture.path().join("proxy");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(/*mode*/ 0o700)).unwrap();
        let endpoint = root.join("control.sock");
        let target = fixture.config().home_directory.join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
        listener.set_nonblocking(/*nonblocking*/ true).unwrap();
        let drop_ack = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let (drop_control, stopping) = (drop_ack.clone(), stop.clone());
        let (sent, acknowledgements) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = UnixListener::from_std(listener).unwrap();
                let mut tick = tokio::time::interval(Duration::from_millis(/*millis*/ 25));
                while !stopping.load(Ordering::SeqCst) {
                    tokio::select! {
                        accepted = listener.accept() => {
                            let (client, _) = accepted.unwrap();
                            let Ok(node) = UnixStream::connect(&target).await else { continue; };
                            let relay = relay(client, node, &drop_control, &sent);
                            tokio::pin!(relay);
                            loop { tokio::select! {
                                _ = &mut relay => break,
                                _ = tick.tick() => { if stopping.load(Ordering::SeqCst) { return; } }
                            } }
                        }
                        _ = tick.tick() => {}
                    }
                }
            });
        });
        Self {
            endpoint,
            drop_ack,
            stop,
            acknowledgements,
            worker: Some(worker),
        }
    }
}
impl Drop for Proxy {
    /// Stops both relay directions and reaps the fixture thread even after an assertion fails.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

/// Relays actual protocol frames; the first connection's Ack is observed but withheld from Node.
async fn relay(
    client: UnixStream,
    node: UnixStream,
    drop_ack: &AtomicBool,
    sent: &mpsc::Sender<EventAckMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut from_controller, mut to_controller) = client.into_split();
    let (mut from_node, mut to_node) = node.into_split();
    let upstream = async {
        while let Some(message) = read_controller_message(&mut from_controller).await? {
            if let ControllerToNodeMessage::EventAck(ack) = &message {
                if drop_ack.load(Ordering::SeqCst) {
                    sent.send(ack.clone())?;
                    continue;
                }
                write_controller_message(&mut to_node, &message).await?;
                sent.send(ack.clone())?;
            } else {
                write_controller_message(&mut to_node, &message).await?;
            }
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    let downstream = async {
        while let Some(message) = read_node_message(&mut from_node).await? {
            write_node_message(&mut to_controller, &message).await?;
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    };
    tokio::select! { result = upstream => result, result = downstream => result }
}

/// Runs the real Controller recovery executable with no file-based business command channel;
/// its API listens on an ephemeral loopback port that these tests never call.
pub(super) fn launch(fixture: &Fixture, proxy: &Proxy) -> ChildGuard {
    let path = fixture.path().join("controller.json");
    fs::write(&path, serde_json::to_vec(&serde_json::json!({
        "controller": {
            "home_directory": fixture.path().join("controller"), "persistence": { "kind": "sqlite" }, "controller_id": "owner",
            "protected_state_directories": [fixture.config().home_directory, fixture.process().host_directory],
            "nodes": [{ "node_id": "test-node", "endpoint": { "kind": "ipc", "path": proxy.endpoint } }],
            "session": { "io_timeout_ms": 5000, "query_interval_ms": 100 }, "reconnect_ms": 100, "timezone": "Asia/Shanghai",
        },
        "api": { "node_id": "test-node" },
        "single_node": null,
    })).unwrap()).unwrap();
    ChildGuard(
        Command::new(
            std::path::Path::new(env!("CARGO_BIN_EXE_ora-node")).with_file_name("ora-controller"),
        )
        .arg("--config")
        .arg(path)
        .args(["--port", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("build ora-controller before running standalone acceptance"),
    )
}

/// Controller death after durable takeover but before Ack cannot lose the result or clone again.
#[test]
fn independent_controller_replays_durable_takeover_after_lost_ack_and_kill() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let clone = configuration(&fixture, &server);
        let home = fixture.path().join("controller");
        let owner = SqliteStore::open(&home, ControllerId::new("owner")).unwrap();
        let command = block_on(owner.accept_request(
            RequestId::new("client-request"),
            request(&server, "unused", "main").payload.spec,
        ))
        .unwrap();
        drop(owner);
        let mut node = super::ipc::launch(&fixture, &clone);
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        let proxy = Proxy::new(&fixture);
        let mut controller = launch(&fixture, &proxy);
        let first = proxy
            .acknowledgements
            .recv_timeout(Duration::from_secs(/*secs*/ 45))
            .unwrap();
        controller.kill();
        let owner = SqliteStore::open(&home, ControllerId::new("owner")).unwrap();
        let result = block_on(owner.result(&command.execution_id))
            .unwrap()
            .expect("Ack requires committed result");
        assert!(matches!(result, ExecutionOutcome::Ready { .. }));
        // The committed result retires the execution from periodic queries.
        assert_eq!(
            block_on(owner.pending_dispatches(&NodeId::new("test-node"))).unwrap(),
            vec![]
        );
        drop(owner);
        server.reject_auth.store(true, Ordering::SeqCst);
        // Draining observations does not acknowledge them; the original Node outbox remains authoritative.
        while proxy.acknowledgements.try_recv().is_ok() {}
        proxy.drop_ack.store(false, Ordering::SeqCst);
        let mut replacement = launch(&fixture, &proxy);
        assert_eq!(
            proxy
                .acknowledgements
                .recv_timeout(Duration::from_secs(/*secs*/ 15))
                .unwrap(),
            first
        );
        let node_path = fixture.config().home_directory.join("ora-node.sqlite3");
        until(|| {
            rusqlite::Connection::open(&node_path)
                .unwrap()
                .query_row("SELECT count(*) FROM clone_outbox", [], |r| {
                    r.get::<_, i64>(/*idx*/ 0)
                })
                .unwrap()
                == 0
        });
        replacement.terminate();
        node.terminate();
        let owner = SqliteStore::open(&home, ControllerId::new("owner")).unwrap();
        assert_eq!(
            block_on(owner.result(&command.execution_id)).unwrap(),
            Some(result)
        );
        let database =
            ora_node_db::NodeDatabase::open(&node_path, fixture.config().identity).unwrap();
        assert_eq!(
            database
                .process_journal()
                .unwrap()
                .attempts(&command.execution_id)
                .unwrap()
                .len(),
            1
        );
        assert!(database.pending_events().unwrap().is_empty());
        assert_eq!(first.execution_id, command.execution_id);
    });
}
