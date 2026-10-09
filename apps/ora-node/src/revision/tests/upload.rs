//! Uploads against a local HTTP object-store double: grant headers, grant requests, the 412 and
//! retry rules, and a restart that resumes a partially uploaded delivery.
use super::super::grants::GrantStore;
use super::super::upload::{HttpUploader, RetryPolicy, UploadEnd, UploadJob, upload};
use super::*;
use ora_node_db::FrozenOutcome;
use ora_node_protocol::*;
use ora_utils::http::{ProxyConfig, ReqwestUploader, UploadOptions};
use pretty_assertions::assert_eq;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const BUNDLE_KEY: &str = "runs/1/revision.bundle";
const HISTORY_KEY: &str = "runs/1/history.jsonl";

/// One PUT the object store received.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Put {
    path: String,
    if_none_match: Option<String>,
    checksum: Option<String>,
    body: Vec<u8>,
}

/// Answers each request with the next scripted status for its path (201 once the script ends).
#[derive(Clone)]
struct ObjectStore {
    address: String,
    received: Arc<Mutex<Vec<Put>>>,
    script: Arc<Mutex<BTreeMap<String, VecDeque<u16>>>>,
}

impl ObjectStore {
    async fn start() -> Self {
        ora_logging::initialize_test_clock();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let store = Self {
            address: listener.local_addr().unwrap().to_string(),
            received: Arc::default(),
            script: Arc::default(),
        };
        let serving = store.clone();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let store = serving.clone();
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
                    let mut lines = head.split("\r\n");
                    let path = lines.next().unwrap().split(' ').nth(1).unwrap().to_owned();
                    let headers: BTreeMap<String, String> = lines
                        .map(|line| {
                            let (name, value) = line.split_once(':').unwrap();
                            (name.to_ascii_lowercase(), value.trim().to_owned())
                        })
                        .collect();
                    let length: usize = headers["content-length"].parse().unwrap();
                    let mut body = buffer[end + 4..].to_vec();
                    while body.len() < length {
                        let mut chunk = [0_u8; 4096];
                        let count = stream.read(&mut chunk).await.unwrap();
                        body.extend_from_slice(&chunk[..count]);
                    }
                    let key = path
                        .split('?')
                        .next()
                        .unwrap()
                        .trim_start_matches('/')
                        .to_owned();
                    let status = store
                        .script
                        .lock()
                        .unwrap()
                        .get_mut(&key)
                        .and_then(VecDeque::pop_front)
                        .unwrap_or(201);
                    store.received.lock().unwrap().push(Put {
                        path: key,
                        if_none_match: headers.get("if-none-match").cloned(),
                        checksum: headers.get("x-amz-checksum-sha256").cloned(),
                        body,
                    });
                    let _ = stream
                        .write_all(
                            format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                                .as_bytes(),
                        )
                        .await;
                });
            }
        });
        store
    }

    /// Queues statuses for the next requests to `key`.
    fn answer(&self, key: &str, statuses: &[u16]) {
        self.script
            .lock()
            .unwrap()
            .entry(key.to_owned())
            .or_default()
            .extend(statuses);
    }

    fn received(&self) -> Vec<Put> {
        self.received.lock().unwrap().clone()
    }

    /// A grant shaped like Cloud's: signed conditional create and checksum headers.
    fn grant(&self, key: &str, valid_for: Duration) -> ObjectUploadGrant {
        ObjectUploadGrant {
            object_key: ObjectKey::new(key),
            url: PresignedUrl::new(format!(
                "http://{}/{key}?X-Amz-Signature=secret",
                self.address
            )),
            method: UploadMethod::Put,
            headers: [
                ("If-None-Match".to_owned(), "*".to_owned()),
                ("x-amz-checksum-sha256".to_owned(), format!("signed-{key}")),
            ]
            .into(),
            expires_at: ora_logging::clock::now_local() + valid_for,
        }
    }
}

/// Short timings keep the tests fast; three attempts per object as in production.
const POLICY: RetryPolicy = RetryPolicy {
    attempts: 3,
    resend: Duration::from_millis(/*millis*/ 50),
    backoff: Duration::from_millis(/*millis*/ 10),
    expiry_margin: Duration::from_secs(/*secs*/ 5),
};

fn uploader() -> HttpUploader {
    HttpUploader::new(
        ReqwestUploader::new(ProxyConfig::default()),
        UploadOptions {
            connect_timeout: Duration::from_secs(/*secs*/ 5),
            total_timeout: Duration::from_secs(/*secs*/ 10),
        },
    )
}

/// A frozen delivered plan with a bundle and a history.
fn frozen(directory: &Path) -> UploadJob {
    std::fs::write(directory.join(BUNDLE_FILE), b"bundle bytes").unwrap();
    std::fs::write(directory.join(HISTORY_FILE), b"{\"type\":\"turnEnded\"}\n").unwrap();
    let object = |key, file| {
        super::super::prepare::stored(&ObjectKey::new(key), &directory.join(file)).unwrap()
    };
    UploadJob {
        operation: OperationId::new("deliver-op"),
        execution: ExecutionId::new("deliver-execution"),
        node_id: NodeId::new("node"),
        directory: directory.to_path_buf(),
        outcome: FrozenOutcome::Delivered(RevisionDelivered {
            node: NodeRuntimeIdentity {
                node_id: NodeId::new("node"),
                incarnation_id: NodeIncarnationId::new("first"),
            },
            final_commit: CommitId::new("89abcdef0123456789abcdef0123456789abcdef"),
            base_commit: CommitId::new("0123456789abcdef0123456789abcdef01234567"),
            revision_ref: RevisionRef::new("refs/ora/revisions/run-1"),
            bundle: object(BUNDLE_KEY, BUNDLE_FILE),
            history: object(HISTORY_KEY, HISTORY_FILE),
        }),
    }
}

/// The checksums every grant request must carry: the frozen digests of what is left.
fn checksums(job: &UploadJob, keys: &[&str]) -> BTreeMap<ObjectKey, Sha256Digest> {
    super::super::upload::objects(&job.outcome)
        .into_iter()
        .filter(|(object, _)| keys.contains(&object.key.as_str()))
        .map(|(object, _)| (object.key.clone(), object.sha256.clone()))
        .collect()
}

/// Hands grants to the store as the control session would.
fn offer(grants: &GrantStore, job: &UploadJob, offered: Vec<ObjectUploadGrant>) {
    grants.offer(UploadGrantMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: job.operation.clone(),
        execution_id: job.execution.clone(),
        payload: UploadGrant { grants: offered },
    });
}

/// Waits for the next grant request the upload sends.
async fn requested(
    requests: &mut tokio::sync::broadcast::Receiver<UploadGrantNeededMessage>,
) -> UploadGrantNeededMessage {
    tokio::time::timeout(Duration::from_secs(/*secs*/ 10), requests.recv())
        .await
        .unwrap()
        .unwrap()
}

/// Without a valid grant the upload asks for one with every remaining frozen digest; an expired
/// grant does not count. The grant's signed headers travel verbatim, and 412 counts as uploaded.
#[tokio::test]
async fn grant_requests_carry_frozen_checksums_and_headers_travel_verbatim() {
    let directory = tempfile::tempdir().unwrap();
    let job = frozen(directory.path());
    let store = ObjectStore::start().await;
    store.answer(HISTORY_KEY, &[412]);
    let grants = GrantStore::new();
    let mut requests = grants.subscribe();
    let running = tokio::spawn({
        let (grants, job) = (grants.clone(), job.clone());
        async move { upload(&uploader(), &grants, &job, POLICY).await }
    });
    let expected = UploadGrantNeededMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: job.operation.clone(),
        execution_id: job.execution.clone(),
        payload: UploadGrantNeeded {
            node_id: NodeId::new("node"),
            checksums: checksums(&job, &[BUNDLE_KEY, HISTORY_KEY]),
        },
    };
    assert_eq!(requested(&mut requests).await, expected);
    // A Controller connecting now gets the same request at once.
    assert_eq!(grants.pending(), vec![expected.clone()]);
    offer(
        &grants,
        &job,
        vec![
            store.grant(BUNDLE_KEY, Duration::from_secs(/*secs*/ 1)),
            store.grant(HISTORY_KEY, Duration::from_secs(/*secs*/ 3600)),
        ],
    );
    // The bundle grant expires inside the safety margin, so the Node asks again.
    assert_eq!(requested(&mut requests).await, expected);
    assert_eq!(store.received(), vec![]);
    offer(
        &grants,
        &job,
        vec![store.grant(BUNDLE_KEY, Duration::from_secs(/*secs*/ 3600))],
    );
    assert_eq!(running.await.unwrap(), UploadEnd::Uploaded);
    assert_eq!(grants.pending(), vec![]);
    let put = |key: &str, body: &[u8]| Put {
        path: key.to_owned(),
        if_none_match: Some("*".into()),
        checksum: Some(format!("signed-{key}")),
        body: body.to_vec(),
    };
    assert_eq!(
        store.received(),
        vec![
            put(BUNDLE_KEY, b"bundle bytes"),
            put(HISTORY_KEY, b"{\"type\":\"turnEnded\"}\n"),
        ]
    );
}

/// Every failed PUT is an attempt; the third failure of one object fails the delivery.
#[tokio::test]
async fn three_failed_attempts_fail_the_upload() {
    let directory = tempfile::tempdir().unwrap();
    let job = frozen(directory.path());
    let store = ObjectStore::start().await;
    store.answer(BUNDLE_KEY, &[500, 503, 500]);
    let grants = GrantStore::new();
    let mut requests = grants.subscribe();
    let running = tokio::spawn({
        let (grants, job) = (grants.clone(), job.clone());
        async move { upload(&uploader(), &grants, &job, POLICY).await }
    });
    requested(&mut requests).await;
    offer(
        &grants,
        &job,
        vec![store.grant(BUNDLE_KEY, Duration::from_secs(/*secs*/ 3600))],
    );
    assert_eq!(running.await.unwrap(), UploadEnd::Failed);
    assert_eq!(
        store
            .received()
            .into_iter()
            .map(|put| put.path)
            .collect::<Vec<_>>(),
        vec![BUNDLE_KEY; 3]
    );
}

/// A refused signature drops the grant and requests a fresh one instead of reusing it.
#[tokio::test]
async fn forbidden_grant_is_replaced() {
    let directory = tempfile::tempdir().unwrap();
    let job = frozen(directory.path());
    let store = ObjectStore::start().await;
    store.answer(BUNDLE_KEY, &[403]);
    let grants = GrantStore::new();
    let mut requests = grants.subscribe();
    let running = tokio::spawn({
        let (grants, job) = (grants.clone(), job.clone());
        async move { upload(&uploader(), &grants, &job, POLICY).await }
    });
    requested(&mut requests).await;
    offer(
        &grants,
        &job,
        vec![
            store.grant(BUNDLE_KEY, Duration::from_secs(/*secs*/ 3600)),
            store.grant(HISTORY_KEY, Duration::from_secs(/*secs*/ 3600)),
        ],
    );
    // After the 403 only the bundle is requested again: the history grant is still held.
    loop {
        let request = requested(&mut requests).await;
        if store.received().len() == 1 {
            assert_eq!(
                request.payload.checksums,
                checksums(&job, &[BUNDLE_KEY, HISTORY_KEY])
            );
            break;
        }
    }
    offer(
        &grants,
        &job,
        vec![store.grant(BUNDLE_KEY, Duration::from_secs(/*secs*/ 3600))],
    );
    assert_eq!(running.await.unwrap(), UploadEnd::Uploaded);
    assert_eq!(store.received().len(), 3);
}

/// A restart after the bundle was stored sends the identical frozen bytes again, accepts the
/// store's 412 for it, and finishes with the declaration frozen before the first PUT; grants from
/// the old process are gone and must be requested again.
#[tokio::test]
async fn restart_after_partial_upload_reuses_frozen_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let job = frozen(directory.path());
    let declaration = job.outcome.clone();
    let store = ObjectStore::start().await;
    let first = GrantStore::new();
    let mut requests = first.subscribe();
    let interrupted = tokio::spawn({
        let (grants, job) = (first.clone(), job.clone());
        async move { upload(&uploader(), &grants, &job, POLICY).await }
    });
    requested(&mut requests).await;
    offer(
        &first,
        &job,
        vec![store.grant(BUNDLE_KEY, Duration::from_secs(/*secs*/ 3600))],
    );
    // The bundle is stored, then the upload waits for a history grant that never comes.
    loop {
        let request = requested(&mut requests).await;
        if request.payload.checksums == checksums(&job, &[HISTORY_KEY]) {
            break;
        }
    }
    interrupted.abort();
    assert!(interrupted.await.unwrap_err().is_cancelled());
    store.answer(BUNDLE_KEY, &[412]);
    let restarted = GrantStore::new();
    let mut requests = restarted.subscribe();
    let resumed = tokio::spawn({
        let (grants, job) = (restarted.clone(), job.clone());
        async move { upload(&uploader(), &grants, &job, POLICY).await }
    });
    assert_eq!(
        requested(&mut requests).await.payload.checksums,
        checksums(&job, &[BUNDLE_KEY, HISTORY_KEY])
    );
    offer(
        &restarted,
        &job,
        vec![
            store.grant(BUNDLE_KEY, Duration::from_secs(/*secs*/ 3600)),
            store.grant(HISTORY_KEY, Duration::from_secs(/*secs*/ 3600)),
        ],
    );
    assert_eq!(resumed.await.unwrap(), UploadEnd::Uploaded);
    let received = store.received();
    assert_eq!(received.len(), 3);
    assert_eq!(received[0], received[1]);
    assert_eq!(job.outcome, declaration);
}

/// Bytes that no longer match the frozen declaration are never uploaded.
#[tokio::test]
async fn changed_frozen_bytes_fail_without_uploading() {
    ora_logging::initialize_test_clock();
    let directory = tempfile::tempdir().unwrap();
    let job = frozen(directory.path());
    std::fs::write(directory.path().join(BUNDLE_FILE), b"other bytes").unwrap();
    let grants = GrantStore::new();
    let mut requests = grants.subscribe();
    assert_eq!(
        upload(&uploader(), &grants, &job, POLICY).await,
        UploadEnd::Failed
    );
    assert!(requests.try_recv().is_err());
}

/// Grants for keys or executions that are not uploading are not kept.
#[tokio::test]
async fn foreign_grants_are_ignored() {
    let directory = tempfile::tempdir().unwrap();
    let job = frozen(directory.path());
    let store = ObjectStore::start().await;
    let grants = GrantStore::new();
    let mut requests = grants.subscribe();
    let running = tokio::spawn({
        let (grants, job) = (grants.clone(), job.clone());
        async move { upload(&uploader(), &grants, &job, POLICY).await }
    });
    requested(&mut requests).await;
    let mut other = job.clone();
    other.execution = ExecutionId::new("other-execution");
    offer(
        &grants,
        &other,
        vec![store.grant(BUNDLE_KEY, Duration::from_secs(/*secs*/ 3600))],
    );
    offer(
        &grants,
        &job,
        vec![store.grant("runs/1/unrelated", Duration::from_secs(/*secs*/ 3600))],
    );
    requested(&mut requests).await;
    assert_eq!(store.received(), vec![]);
    running.abort();
}
