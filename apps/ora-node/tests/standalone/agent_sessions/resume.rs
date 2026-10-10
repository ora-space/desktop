//! A resumed session through the production executable: the Node asks for a download grant over
//! the control connection, downloads and verifies the prior bundle, restores it into the fresh
//! checkout and only then runs the agent.
use super::*;
use pretty_assertions::assert_eq;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const BUNDLE_KEY: &str = "runs/prior/revision.bundle";
/// Appears in every grant URL; it must never reach the Node's ledger.
const SIGNATURE: &str = "X-Amz-Signature=e2e-read-credential";

/// Serves `body` to every GET.
async fn object_store(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let body = body.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                while !buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..count]);
                }
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                response.extend_from_slice(&body);
                let _ = stream.write_all(&response).await;
            });
        }
    });
    address
}

/// Runs Git in `directory`; fixture setup, not the code under test.
fn git(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .current_dir(directory)
        .args([
            "-c",
            "user.name=Prior",
            "-c",
            "user.email=prior@example.test",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

/// A prior run's work, bundled against the base the session's clone has, as its delivery would.
fn prior_bundle(fixture: &Fixture, base: &CommitId) -> (CommitId, Vec<u8>) {
    let prior = fixture.path().join("prior-run");
    let source = fixture.path().join("main");
    git(
        fixture.path(),
        &[
            "clone",
            "-q",
            source.to_str().unwrap(),
            prior.to_str().unwrap(),
        ],
    );
    git(&prior, &["checkout", "-q", base.as_str()]);
    fs::write(prior.join("prior.txt"), "work of the previous run\n").unwrap();
    git(&prior, &["add", "prior.txt"]);
    git(&prior, &["commit", "-q", "-m", "previous run"]);
    git(&prior, &["update-ref", "refs/ora/revisions/prior", "HEAD"]);
    let bundle = fixture.path().join("prior.bundle");
    git(
        &prior,
        &[
            "bundle",
            "create",
            "-q",
            bundle.to_str().unwrap(),
            "refs/ora/revisions/prior",
            &format!("^{}", base.as_str()),
        ],
    );
    (
        CommitId::new(git(&prior, &["rev-parse", "HEAD"])),
        fs::read(bundle).unwrap(),
    )
}

/// The restore asks for its grant over the live connection, verifies the bytes and leaves the
/// clone's branch at the prior final commit before the agent runs; the grant never reaches the
/// ledger and the private download directory is gone afterwards.
#[test]
fn resumed_session_restores_the_prior_revision_with_a_relayed_grant() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let clone = checkout(&fixture);
        let (final_commit, bundle) = prior_bundle(&fixture, &clone.commit);
        let prior = PriorRevision {
            revision_id: RevisionId::new("revision-1"),
            final_commit: final_commit.clone(),
            bundle: StoredObject {
                key: ObjectKey::new(BUNDLE_KEY),
                size: bundle.len() as u64,
                sha256: Sha256Digest::new(ora_utils::hash::sha256_hex(&bundle)),
            },
        };
        let mut child = launch(&fixture, &config);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (requests, ended) = runtime.block_on(async {
            let address = object_store(bundle).await;
            let (mut stream, _) = connect(&fixture).await;
            let mut input = start("hello");
            input.payload.spec.prior_revision = Some(prior);
            send(
                &mut stream,
                ControllerToNodeMessage::StartAgentSession(input.clone()),
            )
            .await;
            let mut requests = Vec::new();
            let mut ending = false;
            loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::DownloadGrantNeeded(request) => {
                        requests.push(request);
                        let grant = ObjectDownloadGrant {
                            object_key: ObjectKey::new(BUNDLE_KEY),
                            url: PresignedUrl::new(format!(
                                "http://{address}/{BUNDLE_KEY}?{SIGNATURE}"
                            )),
                            method: DownloadMethod::Get,
                            headers: Default::default(),
                            expires_at: ora_logging::clock::now_local()
                                + Duration::from_secs(/*secs*/ 3600),
                        };
                        send(
                            &mut stream,
                            ControllerToNodeMessage::DownloadGrant(DownloadGrantMessage {
                                protocol_version: CURRENT_PROTOCOL_VERSION,
                                operation_id: input.operation_id.clone(),
                                execution_id: input.execution_id.clone(),
                                payload: DownloadGrant::Granted {
                                    grants: vec![grant],
                                },
                            }),
                        )
                        .await;
                    }
                    NodeToControllerMessage::ThreadEvent(event) => {
                        send(&mut stream, ack(event.sequence.value())).await;
                        if !ending {
                            ending = true;
                            send(&mut stream, end()).await;
                        }
                    }
                    NodeToControllerMessage::AgentSessionEnded(ended) => {
                        send(&mut stream, ack(ended.sequence.value())).await;
                        break (requests, ended.payload);
                    }
                    NodeToControllerMessage::ExecutionStatus(_)
                    | NodeToControllerMessage::SessionCommandAccepted(_) => {}
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        child.terminate();
        let AgentSessionResult::AgentSessionEnded(ended) = ended;
        let checkout = PathBuf::from(clone.path.as_str());
        assert_eq!(
            (
                ended.reason,
                ended.detail,
                requests
                    .iter()
                    .map(|request| request.payload.node_id.clone())
                    .collect::<Vec<_>>(),
                git(&checkout, &["rev-parse", "HEAD"]),
                git(&checkout, &["symbolic-ref", "HEAD"]),
                fs::read_to_string(checkout.join("prior.txt")).unwrap(),
            ),
            (
                AgentSessionEndReason::UserEnded,
                None,
                vec![NodeId::new("test-node")],
                final_commit.as_str().to_owned(),
                "refs/heads/main".to_owned(),
                "work of the previous run\n".to_owned(),
            )
        );
        let ledger = fs::read(fixture.config().home_directory.join("ora-node.sqlite3")).unwrap();
        assert!(
            !ledger
                .windows(SIGNATURE.len())
                .any(|window| window == SIGNATURE.as_bytes())
        );
        let restores = fixture.config().home_directory.join("revision-restores");
        assert_eq!(fs::read_dir(restores).unwrap().count(), 0);
    });
}
