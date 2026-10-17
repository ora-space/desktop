#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::GitAccessConfig;
use crate::git_server::RetryPolicy;
use crate::revision::GitPolicy;
use gitlancer::{CliGitRunner, GitEnv};
use ora_node_db::{CloneTarget, NodeDatabase, NodeIdentity, SessionCommandInput};
use ora_node_protocol::*;
use ora_test_support::git_server::{GitServer, git};
use pretty_assertions::assert_eq;
use std::time::Duration;

const WORKSPACE: &str = "4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a";
const BINDING: &str = "9d8c7b6a-5f4e-4d3c-8b2a-1f0e9d8c7b6a";

/// Real TLS/Git and SQLite behind the same service and socket used by Agent sessions.
struct Fixture {
    _root: tempfile::TempDir,
    server: GitServer,
    repository: PathBuf,
    service: Arc<OraGitService<CliGitRunner, CliGitRunner>>,
    session: SessionGit,
    database: NodeDatabase,
}

impl Fixture {
    /// Reserves a clone identity and seeds its checkout without a remote.
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let server = GitServer::start(root.path());
        let repository = server.create_repository("tenant/project.git");
        let checkout = root.path().join("checkout");
        fs::create_dir(&checkout).unwrap();
        git(&checkout, &["init", "-q", "-b", "main"]);
        git(
            &checkout,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.test",
                "commit",
                "--allow-empty",
                "-qm",
                "base",
            ],
        );
        let base = CommitId::new(git(&checkout, &["rev-parse", "HEAD"]));
        let spec = CloneExecutionSpec {
            node_id: NodeId::new("node"),
            repository_url: GitRepositoryUrl::parse(&server.url("tenant/project.git")).unwrap(),
            snapshot_ref: SnapshotRef::parse(&format!(
                "refs/ora/upstreams/{BINDING}/snapshots/{}",
                base.as_str()
            ))
            .unwrap(),
            base_commit: base,
            workspace_ref: WorkspaceRef::of(WORKSPACE, "main").unwrap(),
        };
        let mut db = NodeDatabase::open(
            &root.path().join("node.db"),
            NodeIdentity::Require(spec.node_id.clone()),
        )
        .unwrap();
        let clone = ExecutionId::new("clone");
        db.accept_clone(
            &CloneRepositoryMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                request_id: None,
                operation_id: OperationId::new("clone"),
                execution_id: clone.clone(),
                payload: CloneRepository { spec: spec.clone() },
            },
            &CloneTarget {
                repository_id: RepositoryId::new("checkout"),
                root: root.path().to_owned(),
                path: checkout.clone(),
            },
        )
        .unwrap();
        db.accept_session(&session_start(&spec.node_id)).unwrap();
        let material = server.access();
        let service = Arc::new(
            OraGitService::new(OraGitParts {
                directory: root.path().join("sockets"),
                peer_uid: fs::metadata(&checkout).unwrap().uid(),
                workload: DeliveryGit::new(
                    CliGitRunner,
                    GitPolicy {
                        env: GitEnv::automation_defaults()
                            .with_variable("GIT_CONFIG_NOSYSTEM", "1")
                            .with_variable("GIT_CONFIG_GLOBAL", "/dev/null"),
                        config_args: vec!["-c".into(), "core.hooksPath=/dev/null".into()],
                    },
                ),
                node: CliGitRunner,
                access: GitServerAccess::new(
                    GitAccessConfig {
                        ca_cert: material.ca_cert.clone(),
                        client_cert: material.client_cert.clone(),
                        client_key: material.client_key.clone(),
                    },
                    "/usr/bin:/bin".into(),
                    RetryPolicy {
                        delays: [Duration::ZERO; 2],
                    },
                ),
                repository_root: root.path().to_owned(),
                owner: None,
                ledger: db.session_journal().unwrap(),
                shutdown: Shutdown::default(),
            })
            .unwrap(),
        );
        Self {
            _root: root,
            server,
            repository,
            service,
            session: SessionGit {
                execution: ExecutionId::new("agent-session"),
                checkout_execution: clone,
                checkout: ClonedCheckout {
                    path: checkout,
                    spec,
                },
                directory: vec![WorkspaceListing {
                    workspace_id: WORKSPACE.into(),
                    kind: "run".into(),
                    issue_id: Some("issue".into()),
                    issue_title: Some("Issue title".into()),
                    issue_run_id: Some("run".into()),
                    run_status: Some("running".into()),
                }],
            },
            database: db,
        }
    }

    /// Records that the session was asked to end, as the Controller's `EndSession` does.
    fn request_end(&mut self) {
        let start = session_start(&self.session.checkout.spec.node_id);
        self.database
            .accept_session_command(&SessionCommandInput::EndSession(EndSessionMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                operation_id: start.operation_id,
                execution_id: start.execution_id,
                payload: EndSession {
                    node_id: self.session.checkout.spec.node_id.clone(),
                    command_id: CommandId::new("end"),
                    reason: EndSessionReason::Cancelled,
                },
            }))
            .unwrap();
    }

    /// Publishes `commit` of the checkout as `name` on the Git Server, as another writer would.
    fn publish(&self, commit: &str, name: &str) {
        git(
            &self.repository,
            &[
                "fetch",
                "-q",
                self.session.checkout.path.to_str().unwrap(),
                &format!("{commit}:{name}"),
            ],
        );
    }

    /// Invokes the production verb handler with a live session.
    fn run(&self, request: &Request) -> Result<String, operations::Failure> {
        let git = self.service.git.lock().unwrap();
        operations::run(
            &self.service,
            git.as_ref().unwrap(),
            &self.session,
            request,
            &|| true,
        )
    }

    /// Creates a new local commit and returns its ID.
    fn commit(&self, message: &str) -> String {
        git(
            &self.session.checkout.path,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.test",
                "commit",
                "--allow-empty",
                "-qm",
                message,
            ],
        );
        git(&self.session.checkout.path, &["rev-parse", "HEAD"])
    }
}

use std::os::unix::fs::MetadataExt;

/// The session the fixture's socket serves, running in the fixture's checkout.
fn session_start(node: &NodeId) -> StartAgentSessionMessage {
    StartAgentSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("session-op"),
        execution_id: ExecutionId::new("agent-session"),
        payload: StartAgentSession {
            spec: AgentSessionSpec {
                node_id: node.clone(),
                agent_plugin_id: PluginId::new("official/agent"),
                agent_plugin_version: PluginVersion::new("1.0.0"),
                checkout_execution_id: ExecutionId::new("clone"),
                model_binding_id: None,
                git_identity: GitIdentity {
                    name: "User".into(),
                    email: "user@example.com".into(),
                },
                initial_turn: UserTurn {
                    turn_id: TurnId::new("initial"),
                    content: vec![ContentBlock::Text {
                        text: "hello".into(),
                    }],
                },
                prior_revision: None,
                workspace_directory: Vec::new(),
            },
        },
    }
}

/// Requests and responses cross a real Unix socket, bounded in case the server stops answering.
async fn request(path: &Path, line: &[u8]) -> Reply {
    tokio::time::timeout(Duration::from_secs(/*secs*/ 10), async {
        let mut stream = UnixStream::connect(path).await.unwrap();
        // A refusal may be written and the connection closed before the request is read.
        let _ = stream.write_all(line).await;
        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .await
            .unwrap();
        serde_json::from_str(&response).unwrap()
    })
    .await
    .unwrap()
}

/// The socket supplies session identity, validates requests and disappears with its session.
#[tokio::test]
async fn socket_validates_requests_and_closes_with_the_session() {
    let fixture = Fixture::new();
    let socket = fixture.service.open(fixture.session.clone()).unwrap();
    let path = socket.path().to_owned();
    let Reply::Done { output } = request(&path, b"{\"verb\":\"info\"}\n").await else {
        panic!("info failed")
    };
    assert!(output.contains(WORKSPACE));
    let Reply::Failed { code, .. } =
        request(&path, b"{\"verb\":\"info\",\"execution_id\":\"other\"}\n").await
    else {
        panic!("accepted forged identity")
    };
    assert_eq!(code, ErrorCode::Usage);
    let mut idle = UnixStream::connect(&path).await.unwrap();
    drop(socket);
    assert!(!path.exists());
    let mut byte = [0];
    let closed = tokio::time::timeout(Duration::from_secs(/*secs*/ 10), idle.read(&mut byte))
        .await
        .unwrap();
    assert!(
        matches!(closed, Ok(0))
            || closed.is_err_and(|error| error.kind() == io::ErrorKind::ConnectionReset)
    );
    assert!(fixture.server.served().is_empty());
}

/// A connection from a different UID is refused before it can reach Git.
#[tokio::test]
async fn socket_refuses_a_peer_other_than_the_workload_user() {
    let mut fixture = Fixture::new();
    Arc::get_mut(&mut fixture.service).unwrap().peer_uid += 1;
    let socket = fixture.service.open(fixture.session.clone()).unwrap();
    let Reply::Failed { code, .. } = request(socket.path(), b"{\"verb\":\"info\"}\n").await else {
        panic!("accepted wrong UID")
    };
    assert_eq!(code, ErrorCode::Forbidden);
    assert!(fixture.server.served().is_empty());
}

/// Pushes use the durable old value, survive replay, and distinguish rewrites from stale leases.
#[test]
fn push_uses_the_ledger_and_fetch_repairs_a_stale_lease() {
    let fixture = Fixture::new();
    let branch = fixture.session.checkout.spec.workspace_ref.as_str();
    let first = fixture.commit("first");
    fixture.run(&Request::Push { branch: None }).unwrap();
    fixture.run(&Request::Push { branch: None }).unwrap();
    assert_eq!(git(&fixture.repository, &["rev-parse", branch]), first);
    let list = fixture.run(&Request::List {}).unwrap();
    assert!(list.contains("title=\"Issue title\""));
    assert!(list.contains("run=\"run\""));
    // A different writer advanced the remote; the ledger still remembers the first value.
    let second = fixture.commit("second");
    git(
        &fixture.repository,
        &[
            "fetch",
            "-q",
            fixture.session.checkout.path.to_str().unwrap(),
            &format!("main:{branch}"),
        ],
    );
    let third = fixture.commit("third");
    let error = fixture.run(&Request::Push { branch: None }).unwrap_err();
    assert_eq!(error.0, ErrorCode::Conflict);
    assert!(error.1.contains(&second));
    fixture
        .run(&Request::FetchWorkspace {
            workspace_id: WORKSPACE.into(),
            branch: None,
        })
        .unwrap();
    fixture.run(&Request::Push { branch: None }).unwrap();
    assert_eq!(
        fixture
            .service
            .ledger
            .branch_head(
                &fixture.session.checkout_execution,
                &fixture.session.checkout.spec.workspace_ref
            )
            .unwrap(),
        Some(CommitId::new(&third))
    );
    git(&fixture.session.checkout.path, &["reset", "--hard", &first]);
    fixture.commit("divergent");
    let before = fixture.server.served().len();
    assert_eq!(
        fixture.run(&Request::Push { branch: None }).unwrap_err().0,
        ErrorCode::NotFastForward
    );
    assert_eq!(fixture.server.served().len(), before);
    assert_eq!(git(&fixture.repository, &["rev-parse", branch]), third);
    assert!(
        fixture
            .server
            .served()
            .iter()
            .all(|r| r.execution_id.as_deref() == Some("agent-session"))
    );
}

/// Fetch only adds tracking refs; Agent edits, HEAD, index and configuration remain intact.
#[test]
fn fetch_preserves_local_work_and_ignores_untrusted_tracking_refs() {
    let fixture = Fixture::new();
    let branch = fixture.session.checkout.spec.workspace_ref.as_str();
    let pushed = fixture.commit("published");
    fixture.run(&Request::Push { branch: None }).unwrap();
    let checkout = &fixture.session.checkout.path;
    let tracking = format!("refs/remotes/ora/workspaces/{WORKSPACE}/main");
    git(
        checkout,
        &[
            "update-ref",
            &tracking,
            fixture.session.checkout.spec.base_commit.as_str(),
        ],
    );
    fs::write(checkout.join("local.txt"), "uncommitted\n").unwrap();
    let before = git(checkout, &["status", "--porcelain"]);
    fixture
        .run(&Request::FetchWorkspace {
            workspace_id: WORKSPACE.into(),
            branch: Some("main".into()),
        })
        .unwrap();
    assert_eq!(git(checkout, &["rev-parse", "HEAD"]), pushed);
    assert_eq!(git(checkout, &["status", "--porcelain"]), before);
    assert_eq!(git(checkout, &["remote"]), "");
    assert_eq!(
        git(checkout, &["rev-parse", &tracking]),
        git(&fixture.repository, &["rev-parse", branch])
    );
    let git = fixture.service.git.lock().unwrap();
    assert_eq!(
        operations::run(
            &fixture.service,
            git.as_ref().unwrap(),
            &fixture.session,
            &Request::List {},
            &|| false
        )
        .unwrap_err()
        .0,
        ErrorCode::NoSession
    );
}

/// Mirrors the managed runner's runtime ownership without starting a host process.
struct RuntimeRunner {
    _runtime: tokio::runtime::Runtime,
}

impl GitRunner for RuntimeRunner {
    /// This lifetime regression must not execute Git.
    fn run(
        &self,
        _command: &gitlancer::GitCommand,
    ) -> Result<gitlancer::GitOutput, gitlancer::GitExecError> {
        panic!("lifetime-only runner")
    }
}

/// The final broker handle can disappear on a Tokio task after session shutdown. Its synchronous
/// runner runtimes must be retired without panicking inside that async context.
#[tokio::test]
async fn dropping_the_service_from_an_async_task_retires_managed_runtimes() {
    let fixture = Fixture::new();
    let service = OraGitService::new(OraGitParts {
        directory: fixture._root.path().join("runtime-sockets"),
        peer_uid: fixture.service.peer_uid,
        workload: DeliveryGit::new(
            CliGitRunner,
            GitPolicy {
                env: GitEnv::automation_defaults(),
                config_args: vec![],
            },
        ),
        node: RuntimeRunner {
            _runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        },
        access: fixture.service.access.clone(),
        repository_root: fixture.service.repository_root.clone(),
        owner: None,
        ledger: fixture.service.ledger.clone(),
        shutdown: Shutdown::default(),
    })
    .unwrap();
    drop(service);
}

/// A push of the current branch through the session socket.
const PUSH: &[u8] = b"{\"verb\":\"push\"}\n";

/// Once the session is asked to end, requests fail as `no_session` before reaching the Git
/// Server, even though the socket is still open.
#[tokio::test(flavor = "multi_thread")]
async fn requests_fail_once_an_end_is_requested() {
    let mut fixture = Fixture::new();
    fixture.commit("work");
    let socket = fixture.service.open(fixture.session.clone()).unwrap();
    assert!(matches!(
        request(socket.path(), PUSH).await,
        Reply::Done { .. }
    ));
    let served = fixture.server.served().len();
    fixture.request_end();
    let Reply::Failed { code, .. } = request(socket.path(), PUSH).await else {
        panic!("a cancelled session pushed")
    };
    assert_eq!(code, ErrorCode::NoSession);
    assert_eq!(fixture.server.served().len(), served);
}

/// While one request's Git runs, another is `busy`; closing the socket refuses new requests at
/// once but waits for the running one, whose answer still reaches its client.
#[tokio::test(flavor = "multi_thread")]
async fn closing_waits_for_the_request_in_flight() {
    let fixture = Fixture::new();
    let pushed = fixture.commit("work");
    let socket = fixture.service.open(fixture.session.clone()).unwrap();
    let path = socket.path().to_owned();
    fixture.server.pause(true);
    let in_flight = tokio::spawn({
        let path = path.clone();
        async move {
            let mut stream = UnixStream::connect(&path).await.unwrap();
            stream.write_all(PUSH).await.unwrap();
            let mut response = String::new();
            BufReader::new(stream)
                .read_line(&mut response)
                .await
                .unwrap();
            serde_json::from_str::<Reply>(&response).unwrap()
        }
    });
    // The push borrows the checkout's objects right before it reaches the Git Server.
    let transports = fixture.service.repository_root.join(".ora-transport");
    let borrowing = || {
        fs::read_dir(&transports).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().join("objects/info/alternates").exists())
        })
    };
    while !borrowing() {
        tokio::time::sleep(Duration::from_millis(/*millis*/ 20)).await;
    }
    tokio::time::sleep(Duration::from_millis(/*millis*/ 300)).await;
    let Reply::Failed { code: busy, .. } = request(&path, PUSH).await else {
        panic!("a second request ran concurrently")
    };
    let closing = tokio::spawn(socket.close());
    tokio::time::sleep(Duration::from_millis(/*millis*/ 300)).await;
    let Reply::Failed { code: closed, .. } = request(&path, PUSH).await else {
        panic!("a closing socket accepted a request")
    };
    assert!(!closing.is_finished());
    assert!(path.exists());
    fixture.server.pause(false);
    let answer = tokio::time::timeout(Duration::from_secs(/*secs*/ 30), in_flight)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(/*secs*/ 30), closing)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (busy, closed, answer, path.exists()),
        (
            ErrorCode::Busy,
            ErrorCode::NoSession,
            Reply::Done {
                output: format!(
                    "{} {pushed}\n",
                    fixture.session.checkout.spec.workspace_ref.as_str()
                )
            },
            false
        )
    );
}

/// `fetch upstream` maps the binding's upstream heads to `refs/remotes/ora/upstream/`; with none
/// published there is nothing to fetch.
#[test]
fn fetch_upstream_reads_the_binding_heads() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.run(&Request::FetchUpstream {}).unwrap_err().0,
        ErrorCode::NotFound
    );
    let upstream = fixture.commit("upstream");
    git(
        &fixture.session.checkout.path,
        &["reset", "-q", "--hard", "HEAD~1"],
    );
    fixture.publish(
        &upstream,
        &format!("refs/ora/upstreams/{BINDING}/heads/main"),
    );
    fixture.publish(&upstream, "refs/ora/upstreams/other/heads/main");
    let output = fixture.run(&Request::FetchUpstream {}).unwrap();
    let checkout = &fixture.session.checkout.path;
    assert_eq!(
        (
            output,
            git(
                checkout,
                &[
                    "for-each-ref",
                    "--format=%(refname) %(objectname)",
                    "refs/remotes/"
                ]
            ),
            git(checkout, &["rev-parse", "HEAD"]),
        ),
        (
            format!("refs/remotes/ora/upstream/main {upstream}\n"),
            format!("refs/remotes/ora/upstream/main {upstream}"),
            fixture
                .session
                .checkout
                .spec
                .base_commit
                .as_str()
                .to_owned(),
        )
    );
}

/// A local branch that does not exist, and a Workspace with no branches, are `not_found`.
#[test]
fn missing_branches_are_not_found() {
    let fixture = Fixture::new();
    let served = fixture.server.served().len();
    let push = fixture
        .run(&Request::Push {
            branch: Some("missing".into()),
        })
        .unwrap_err()
        .0;
    assert_eq!(fixture.server.served().len(), served);
    let fetch = fixture
        .run(&Request::FetchWorkspace {
            workspace_id: "0f0e0d0c-0b0a-4908-8706-050403020100".into(),
            branch: None,
        })
        .unwrap_err()
        .0;
    let single = fixture
        .run(&Request::FetchWorkspace {
            workspace_id: WORKSPACE.into(),
            branch: Some("main".into()),
        })
        .unwrap_err()
        .0;
    assert_eq!(
        [push, fetch, single],
        [
            ErrorCode::NotFound,
            ErrorCode::NotFound,
            ErrorCode::NotFound
        ]
    );
}

/// After a push whose ledger update was lost, the next push sees a stale lease with the Git
/// Server already at the commit and succeeds; a push never touches HEAD, the index or the
/// worktree.
#[test]
fn a_stale_lease_already_at_the_commit_is_success_and_pushes_leave_the_checkout() {
    let fixture = Fixture::new();
    let checkout = &fixture.session.checkout.path;
    let reference = &fixture.session.checkout.spec.workspace_ref;
    let commit = fixture.commit("pushed before the ledger learned it");
    fixture.publish(&commit, reference.as_str());
    fs::write(checkout.join("staged.txt"), "staged\n").unwrap();
    git(checkout, &["add", "staged.txt"]);
    fs::write(checkout.join("loose.txt"), "loose\n").unwrap();
    let state = |checkout: &Path| {
        (
            git(checkout, &["symbolic-ref", "HEAD"]),
            git(checkout, &["rev-parse", "HEAD"]),
            git(checkout, &["status", "--porcelain"]),
            git(checkout, &["for-each-ref", "refs/heads/"]),
        )
    };
    let before = state(checkout);
    let output = fixture.run(&Request::Push { branch: None }).unwrap();
    assert_eq!(
        (
            output,
            state(checkout),
            fixture
                .service
                .ledger
                .branch_head(&fixture.session.checkout_execution, reference)
                .unwrap(),
            git(
                checkout,
                &[
                    "rev-parse",
                    &format!("refs/remotes/ora/workspaces/{WORKSPACE}/main")
                ]
            ),
        ),
        (
            format!("{} {commit}\n", reference.as_str()),
            before,
            Some(CommitId::new(&commit)),
            commit.clone(),
        )
    );
}

/// The service empties its socket directory of whatever an earlier Node process left.
#[test]
fn the_socket_directory_is_cleared_when_the_service_starts() {
    let fixture = Fixture::new();
    let directory = fixture._root.path().join("stale-sockets");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("old.sock"), "").unwrap();
    let service = OraGitService::new(OraGitParts {
        directory: directory.clone(),
        peer_uid: fixture.service.peer_uid,
        workload: DeliveryGit::new(
            CliGitRunner,
            GitPolicy {
                env: GitEnv::automation_defaults(),
                config_args: vec![],
            },
        ),
        node: CliGitRunner,
        access: fixture.service.access.clone(),
        repository_root: fixture.service.repository_root.clone(),
        owner: None,
        ledger: fixture.service.ledger.clone(),
        shutdown: Shutdown::default(),
    })
    .unwrap();
    assert_eq!(
        (
            fs::read_dir(&directory).unwrap().count(),
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        ),
        (0, 0o711)
    );
    drop(service);
}

/// Settings the Agent can write into the checkout never reach the Node's Git Server access:
/// no helper runs, TLS stays verified, the URL is not rewritten and the execution header is
/// the session's. A checkout borrowing objects from elsewhere is refused before any request.
#[tokio::test(flavor = "multi_thread")]
async fn hostile_checkout_configuration_has_no_effect_through_the_socket() {
    let fixture = Fixture::new();
    let checkout = &fixture.session.checkout.path;
    let marker = fixture._root.path().join("helper-ran");
    let helper = fixture._root.path().join("helper.sh");
    fs::write(
        &helper,
        format!("#!/bin/sh\ntouch '{}'\necho password=x\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(/*mode*/ 0o755)).unwrap();
    let url = fixture
        .session
        .checkout
        .spec
        .repository_url
        .as_str()
        .to_owned();
    for (key, value) in [
        ("credential.helper", format!("!{}", helper.display())),
        ("core.askPass", helper.display().to_string()),
        (&format!("http.{url}.sslVerify"), "false".to_owned()),
        (
            &format!("http.{url}.extraHeader"),
            "Ora-Execution-Id: forged".to_owned(),
        ),
        ("url.https://elsewhere.invalid/.insteadOf", url.clone()),
    ] {
        git(checkout, &["config", key, &value]);
    }
    let pushed = fixture.commit("work");
    let socket = fixture.service.open(fixture.session.clone()).unwrap();
    let push = request(socket.path(), PUSH).await;
    let fetch = request(
        socket.path(),
        format!("{{\"verb\":\"fetch_workspace\",\"workspace_id\":\"{WORKSPACE}\"}}\n").as_bytes(),
    )
    .await;
    assert!(matches!(push, Reply::Done { .. }), "{push:?}");
    assert!(matches!(fetch, Reply::Done { .. }), "{fetch:?}");
    assert_eq!(
        git(
            &fixture.repository,
            &[
                "rev-parse",
                fixture.session.checkout.spec.workspace_ref.as_str()
            ]
        ),
        pushed
    );
    assert!(!marker.exists());
    assert!(
        fixture
            .server
            .served()
            .iter()
            .all(|served| served.execution_id.as_deref() == Some("agent-session"))
    );
    let served = fixture.server.served().len();
    let elsewhere = fixture._root.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    fs::write(
        checkout.join(".git/objects/info/alternates"),
        format!("{}\n", elsewhere.display()),
    )
    .unwrap();
    let Reply::Failed { code, .. } = request(socket.path(), PUSH).await else {
        panic!("pushed from a checkout with alternates")
    };
    assert_eq!(
        (code, fixture.server.served().len()),
        (ErrorCode::Internal, served)
    );
}
