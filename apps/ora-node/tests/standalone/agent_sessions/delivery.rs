//! Revision delivery through the production executable: a real session ends, the delivery
//! snapshots its checkout, asks for grants, uploads to a local object store and reports.
use super::*;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const BUNDLE_KEY: &str = "runs/e2e/revision.bundle";
const HISTORY_KEY: &str = "runs/e2e/history.jsonl";
/// Appears in every grant URL; it must never reach the Node's ledger.
const SIGNATURE: &str = "X-Amz-Signature=e2e-bearer-credential";

/// Stores every PUT body by key and answers 201.
async fn object_store() -> (String, Arc<Mutex<BTreeMap<String, Vec<u8>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let objects = Arc::new(Mutex::new(BTreeMap::new()));
    let stored = objects.clone();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let stored = stored.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let end = loop {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..count]);
                    if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end;
                    }
                };
                let head = String::from_utf8(buffer[..end].to_vec()).unwrap();
                let path = head.split(' ').nth(1).unwrap();
                let key = path.split('?').next().unwrap().trim_start_matches('/');
                let length: usize = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body = buffer[end + 4..].to_vec();
                while body.len() < length {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).await.unwrap();
                    body.extend_from_slice(&chunk[..count]);
                }
                stored.lock().unwrap().insert(key.to_owned(), body);
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 201 Created\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    )
                    .await;
            });
        }
    });
    (address, objects)
}

/// Waits for the next non-heartbeat message. Preparation runs several host-managed Git Runs,
/// so this deadline is longer than the session fixture's.
async fn next(stream: &mut tokio::net::UnixStream) -> NodeToControllerMessage {
    tokio::time::timeout(std::time::Duration::from_secs(/*secs*/ 90), async {
        loop {
            let message = read_node_message(stream)
                .await
                .unwrap()
                .expect("Node disconnected");
            if !matches!(message, NodeToControllerMessage::Heartbeat(_)) {
                return message;
            }
        }
    })
    .await
    .expect("Node delivery deadline")
}

/// Delivers the session fixture's execution from its clone.
fn deliver(base: &CommitId) -> DeliverRevisionMessage {
    DeliverRevisionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("deliver-operation"),
        execution_id: ExecutionId::new("deliver-execution"),
        payload: DeliverRevision {
            spec: DeliverRevisionSpec {
                node_id: NodeId::new("test-node"),
                session_execution_id: ExecutionId::new(EXECUTION),
                checkout_execution_id: start("").payload.spec.checkout_execution_id,
                base_commit: base.clone(),
                revision_ref: RevisionRef::new("refs/ora/revisions/e2e"),
                bundle_key: ObjectKey::new(BUNDLE_KEY),
                history_key: ObjectKey::new(HISTORY_KEY),
                prior_revision: None,
            },
        },
    }
}

/// An ended session's uncommitted file is delivered as a bundle plus the sealed history, using
/// grants requested with the frozen checksums; grants never reach the ledger.
#[test]
fn ended_session_is_delivered_with_requested_grants() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let clone = checkout(&fixture);
        let mut child = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (result, objects) = runtime.block_on(async {
            let (address, objects) = object_store().await;
            let (mut stream, _) = connect(&fixture).await;
            send(
                &mut stream,
                ControllerToNodeMessage::StartAgentSession(start("hello")),
            )
            .await;
            send(&mut stream, end()).await;
            loop {
                if let NodeToControllerMessage::AgentSessionEnded(_) = receive(&mut stream).await {
                    break;
                }
            }
            fs::write(
                PathBuf::from(clone.path.as_str()).join("delivered.txt"),
                "left uncommitted by the run\n",
            )
            .unwrap();
            let input = deliver(&clone.commit);
            send(
                &mut stream,
                ControllerToNodeMessage::DeliverRevision(input.clone()),
            )
            .await;
            loop {
                match next(&mut stream).await {
                    NodeToControllerMessage::UploadGrantNeeded(request) => {
                        assert_eq!(
                            request
                                .payload
                                .checksums
                                .keys()
                                .cloned()
                                .collect::<Vec<_>>(),
                            vec![ObjectKey::new(HISTORY_KEY), ObjectKey::new(BUNDLE_KEY)]
                        );
                        let grants = [BUNDLE_KEY, HISTORY_KEY]
                            .into_iter()
                            .map(|key| ObjectUploadGrant {
                                object_key: ObjectKey::new(key),
                                url: PresignedUrl::new(format!(
                                    "http://{address}/{key}?{SIGNATURE}"
                                )),
                                method: UploadMethod::Put,
                                headers: [("If-None-Match".to_owned(), "*".to_owned())].into(),
                                expires_at: ora_logging::clock::now_local()
                                    + std::time::Duration::from_secs(/*secs*/ 3600),
                            })
                            .collect();
                        send(
                            &mut stream,
                            ControllerToNodeMessage::UploadGrant(UploadGrantMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                operation_id: input.operation_id.clone(),
                                execution_id: input.execution_id.clone(),
                                payload: UploadGrant { grants },
                            }),
                        )
                        .await;
                    }
                    NodeToControllerMessage::RevisionResult(result) => {
                        assert_eq!(result.sequence, Sequence::new(/*value*/ 1));
                        send(
                            &mut stream,
                            ControllerToNodeMessage::EventAck(EventAckMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                operation_id: input.operation_id.clone(),
                                execution_id: input.execution_id.clone(),
                                sequence: result.sequence,
                                payload: EventAck {
                                    node_id: NodeId::new("test-node"),
                                },
                            }),
                        )
                        .await;
                        let objects = objects.lock().unwrap().clone();
                        break (result.payload, objects);
                    }
                    NodeToControllerMessage::ExecutionStatus(_)
                    | NodeToControllerMessage::ThreadEvent(_)
                    | NodeToControllerMessage::AgentSessionEnded(_) => {}
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        child.terminate();
        let RevisionExecutionResult::RevisionDelivered(delivered) = result else {
            panic!("expected a delivered Revision, got {result:?}");
        };
        let measured = |key: &str| StoredObject {
            key: ObjectKey::new(key),
            size: objects[key].len() as u64,
            sha256: Sha256Digest::new(ora_utils::hash::sha256_hex(&objects[key])),
        };
        assert_eq!(
            (delivered.bundle.clone(), delivered.history.clone()),
            (measured(BUNDLE_KEY), measured(HISTORY_KEY))
        );
        assert_eq!(
            objects[HISTORY_KEY],
            fs::read(
                ora_history::history_path(
                    &fixture.config().home_directory.join("sessions"),
                    EXECUTION
                )
                .unwrap()
            )
            .unwrap()
        );
        assert_eq!(delivered.base_commit, clone.commit);
        assert_ne!(delivered.final_commit, clone.commit);
        let ledger = fs::read(fixture.config().home_directory.join("ora-node.sqlite3")).unwrap();
        assert!(
            !ledger
                .windows(SIGNATURE.len())
                .any(|window| window == SIGNATURE.as_bytes())
        );
        let frozen = fixture.config().home_directory.join("revision-deliveries");
        assert_eq!(fs::read_dir(frozen).unwrap().count(), 0);
    });
}
