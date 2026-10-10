//! The production restorer against a local HTTP object-store double and real repositories: the
//! grant exchange, verification of the downloaded bytes, the retry and renewal rules, the
//! deadline, and the Node-private directory that never outlives a restore.
use super::super::{DownloadGrants, HttpDownloader, RESTORE_ROOT, RestorePolicy, RevisionRestorer};
use super::*;
use crate::session::{PriorRevisionRestore, RestoreFailure, RestoreRequest, Restored};
use ora_utils::http::{FetchOptions, ProxyConfig, ReqwestFetcher};
use pretty_assertions::assert_eq;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serves `body` to every GET, after the scripted statuses run out.
#[derive(Clone)]
struct ObjectStore {
    address: String,
    body: Arc<Vec<u8>>,
    script: Arc<Mutex<VecDeque<u16>>>,
    /// The signature header of every request, in order.
    received: Arc<Mutex<Vec<Option<String>>>>,
}

impl ObjectStore {
    /// Starts serving on a free local port.
    async fn start(body: Vec<u8>, statuses: &[u16]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let store = Self {
            address: listener.local_addr().unwrap().to_string(),
            body: Arc::new(body),
            script: Arc::new(Mutex::new(statuses.iter().copied().collect())),
            received: Arc::default(),
        };
        let serving = store.clone();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let store = serving.clone();
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                        let mut chunk = [0_u8; 1024];
                        let count = stream.read(&mut chunk).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..count]);
                    }
                    let head = String::from_utf8_lossy(&buffer).to_ascii_lowercase();
                    let proof = head
                        .lines()
                        .find_map(|line| line.strip_prefix("x-amz-meta-proof:"))
                        .map(|value| value.trim().to_owned());
                    store.received.lock().unwrap().push(proof);
                    let status = store.script.lock().unwrap().pop_front().unwrap_or(200);
                    let body: &[u8] = if status == 200 { &store.body } else { b"" };
                    let mut response = format!(
                        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(body);
                    let _ = stream.write_all(&response).await;
                });
            }
        });
        store
    }

    /// A grant shaped like Cloud's, with a signed header the store sees.
    fn grant(&self, key: &ObjectKey, serial: usize) -> ObjectDownloadGrant {
        ObjectDownloadGrant {
            object_key: key.clone(),
            url: PresignedUrl::new(format!(
                "http://{}/{}?X-Amz-Signature=secret",
                self.address,
                key.as_str()
            )),
            method: DownloadMethod::Get,
            headers: [("x-amz-meta-proof".to_owned(), format!("grant-{serial}"))].into(),
            expires_at: ora_logging::clock::now_local() + Duration::from_secs(/*secs*/ 3600),
        }
    }

    /// The signed header of every request so far.
    fn received(&self) -> Vec<Option<String>> {
        self.received.lock().unwrap().clone()
    }
}

/// Short timings keep the tests fast; three retries as in production.
const POLICY: RestorePolicy = RestorePolicy {
    deadline: Duration::from_secs(/*secs*/ 20),
    retries: 3,
    resend: Duration::from_millis(/*millis*/ 50),
    backoff: Duration::from_millis(/*millis*/ 10),
    expiry_margin: Duration::from_secs(/*secs*/ 5),
};

/// The restorer under test, with Node home `home`.
fn restorer(
    grants: &DownloadGrants,
    home: &Path,
    policy: RestorePolicy,
) -> RevisionRestorer<CliGitRunner, HttpDownloader> {
    RevisionRestorer::new(
        delivery_git(),
        HttpDownloader::new(
            ReqwestFetcher::new(ProxyConfig::default()),
            FetchOptions {
                connect_timeout: Duration::from_secs(/*secs*/ 5),
                total_timeout: Duration::from_secs(/*secs*/ 10),
            },
        ),
        grants.clone(),
        NodeId::new("node"),
        home.join(RESTORE_ROOT),
        /*owner*/ None,
        policy,
    )
}

/// Answers every download grant request the way the Controller would, with a fresh grant or, when
/// `refuse`, a refusal; returns the requests it saw.
fn controller(
    grants: &DownloadGrants,
    store: &ObjectStore,
    key: ObjectKey,
    refuse: bool,
) -> Arc<Mutex<Vec<DownloadGrantNeededMessage>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut requests = grants.subscribe();
    let (grants, store, recorded) = (grants.clone(), store.clone(), Arc::clone(&seen));
    tokio::spawn(async move {
        while let Ok(request) = requests.recv().await {
            let serial = {
                let mut seen = recorded.lock().unwrap();
                seen.push(request.clone());
                seen.len()
            };
            let payload = if refuse {
                DownloadGrant::Refused {}
            } else {
                DownloadGrant::Granted {
                    grants: vec![store.grant(&key, serial)],
                }
            };
            grants.offer(DownloadGrantMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: request.operation_id,
                execution_id: request.execution_id,
                payload,
            });
        }
    });
    seen
}

/// The request the session sends for the fixture's restore.
fn request(world: &World, checkout: PathBuf) -> RestoreRequest {
    RestoreRequest {
        operation: OperationId::new("run-2"),
        execution: ExecutionId::new("session-2"),
        checkout,
        prior: world.prior.clone(),
    }
}

/// A granted download is verified and restored; its Node-private directory is gone afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn restores_through_a_granted_download_and_removes_its_directory() {
    let world = World::new();
    let checkout = world.clone_into("fresh");
    let home = world.path("home");
    fs::create_dir(&home).unwrap();
    let store = ObjectStore::start(fs::read(world.bundle()).unwrap(), &[]).await;
    let grants = DownloadGrants::new();
    let seen = controller(
        &grants,
        &store,
        world.prior.bundle.key.clone(),
        /*refuse*/ false,
    );
    let restored = restorer(&grants, &home, POLICY)
        .restore(request(&world, checkout.clone()))
        .await;
    assert_eq!(restored, Ok(Restored::OnRemoteHistory));
    assert_eq!(
        git(&checkout, &["rev-parse", "HEAD"]),
        world.prior.final_commit.as_str()
    );
    assert_eq!(
        seen.lock().unwrap().clone(),
        vec![DownloadGrantNeededMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: OperationId::new("run-2"),
            execution_id: ExecutionId::new("session-2"),
            payload: DownloadGrantNeeded {
                node_id: NodeId::new("node"),
            },
        }]
    );
    assert_eq!(store.received(), vec![Some("grant-1".into())]);
    assert_eq!(
        fs::read_dir(home.join(RESTORE_ROOT)).unwrap().count(),
        0,
        "the private download directory is removed"
    );
    assert_eq!(grants.pending(), vec![]);
}

/// Server errors are retried with the same grant, and a 403 renews the grant exactly once.
#[tokio::test(flavor = "multi_thread")]
async fn retries_server_errors_and_renews_a_forbidden_grant_once() {
    let world = World::new();
    let home = world.path("home");
    fs::create_dir(&home).unwrap();
    let bundle = fs::read(world.bundle()).unwrap();

    let store = ObjectStore::start(bundle.clone(), &[503, 403, 500]).await;
    let grants = DownloadGrants::new();
    let seen = controller(
        &grants,
        &store,
        world.prior.bundle.key.clone(),
        /*refuse*/ false,
    );
    let restored = restorer(&grants, &home, POLICY)
        .restore(request(&world, world.clone_into("renewed")))
        .await;
    assert_eq!(restored, Ok(Restored::OnRemoteHistory));
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(
        store.received(),
        vec![
            Some("grant-1".into()),
            Some("grant-1".into()),
            Some("grant-2".into()),
            Some("grant-2".into()),
        ]
    );

    let store = ObjectStore::start(bundle.clone(), &[403, 403]).await;
    let grants = DownloadGrants::new();
    controller(
        &grants,
        &store,
        world.prior.bundle.key.clone(),
        /*refuse*/ false,
    );
    let restored = restorer(&grants, &home, POLICY)
        .restore(request(&world, world.clone_into("forbidden")))
        .await;
    assert_eq!(restored, Err(RestoreFailure::Unavailable));

    let store = ObjectStore::start(bundle, &[500, 502, 503, 504]).await;
    let grants = DownloadGrants::new();
    controller(
        &grants,
        &store,
        world.prior.bundle.key.clone(),
        /*refuse*/ false,
    );
    let restored = restorer(&grants, &home, POLICY)
        .restore(request(&world, world.clone_into("exhausted")))
        .await;
    assert_eq!(restored, Err(RestoreFailure::Unavailable));
    assert_eq!(
        store.received().len(),
        4,
        "three retries after the first attempt"
    );
}

/// A refusal, bytes that do not match the declared digest, and a grant that never comes all fail
/// the restore as unavailable without touching the checkout.
#[tokio::test(flavor = "multi_thread")]
async fn refusal_mismatch_and_deadline_are_unavailable() {
    let world = World::new();
    let home = world.path("home");
    fs::create_dir(&home).unwrap();
    let checkout = world.clone_into("fresh");
    let before = state(&checkout);
    let bundle = fs::read(world.bundle()).unwrap();

    let store = ObjectStore::start(bundle.clone(), &[]).await;
    let grants = DownloadGrants::new();
    controller(
        &grants,
        &store,
        world.prior.bundle.key.clone(),
        /*refuse*/ true,
    );
    let restored = restorer(&grants, &home, POLICY)
        .restore(request(&world, checkout.clone()))
        .await;
    assert_eq!(restored, Err(RestoreFailure::Unavailable));
    assert_eq!(
        store.received(),
        vec![],
        "a refused restore downloads nothing"
    );

    let mut tampered = bundle;
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    let store = ObjectStore::start(tampered, &[]).await;
    let grants = DownloadGrants::new();
    controller(
        &grants,
        &store,
        world.prior.bundle.key.clone(),
        /*refuse*/ false,
    );
    let restored = restorer(&grants, &home, POLICY)
        .restore(request(&world, checkout.clone()))
        .await;
    assert_eq!(restored, Err(RestoreFailure::Unavailable));

    let grants = DownloadGrants::new();
    let short = RestorePolicy {
        deadline: Duration::from_millis(/*millis*/ 300),
        ..POLICY
    };
    let restored = restorer(&grants, &home, short)
        .restore(request(&world, checkout.clone()))
        .await;
    assert_eq!(restored, Err(RestoreFailure::Unavailable));
    assert_eq!(
        grants.pending(),
        vec![],
        "an abandoned restore asks for nothing more"
    );
    assert_eq!(state(&checkout), before);
    assert_eq!(fs::read_dir(home.join(RESTORE_ROOT)).unwrap().count(), 0);
}
