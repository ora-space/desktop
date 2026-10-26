use crate::{AgentConfig, ProcessConfig, ServiceConfig, SessionWorkload, Shutdown};
use ora_process::ProcessIdentity;
use pretty_assertions::assert_eq;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod ora_git;

/// A service configuration over `root` that would run agents as `workload_uid` from
/// `workload_directory`; everything else is valid and never reached by these tests.
fn config(
    root: &Path,
    workload_uid: Option<u32>,
    workload_directory: Option<PathBuf>,
) -> ServiceConfig {
    let mut config: ServiceConfig = serde_json::from_value(serde_json::json!({
        "node": {
            "home_directory": root.join("node"),
            "identity": "Discover",
            "repositories": [],
        },
        "process": {
            "host_directory": root.join("host"),
            "expected_uid": 0,
            "git_program": "/usr/bin/git",
            "environment": {},
            "command_timeout_ms": 1000,
            "cleanup_timeout_ms": 1000,
            "shutdown_grace_ms": 1000,
        },
        "recovery_interval_ms": 50,
        "timezone": "UTC",
    }))
    .expect("service configuration");
    config.process.workload_uid = workload_uid;
    config.agent = Some(AgentConfig {
        model_proxy: None,
        deno_path: PathBuf::from("/usr/bin/deno"),
        ready_timeout_ms: 1000,
        workload_directory,
        ora_git_directory: None,
    });
    config
}

/// Creates `path` as a directory with exactly `mode`.
fn directory(path: &Path, mode: u32) -> PathBuf {
    fs::create_dir_all(path).expect("create directory");
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set mode");
    path.to_path_buf()
}

/// Startup keeps unknown restore inputs; local cleanup failure never rewrites stop evidence.
#[test]
fn restore_cleanup_requires_original_terminal_stop_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    use ora_node_db::{NodeIdentity, ProcessAttempt};
    use ora_node_protocol::*;
    use ora_process_protocol::*;
    ora_logging::initialize_test_clock();
    let subscriber = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    );
    tracing::dispatcher::with_default(&subscriber, || {
        let root = tempfile::tempdir()?;
        let mut deployment = config(
            root.path(),
            /*workload_uid*/ None,
            /*workload_directory*/ None,
        );
        deployment.node.identity = NodeIdentity::Require(NodeId::new("node"));
        let bin = directory(&root.path().join("bin"), 0o755);
        let git_config = root.path().join("gitconfig");
        fs::write(&git_config, "")?;
        let mut node = crate::Node::open(deployment.node, deployment.process, Shutdown::default())?;
        // Configured exactly as a deployed Node is, so the transport repository below is the real
        // restore location and not a path the test invented.
        node.configure_clone(crate::CloneConfig {
            repository_root: directory(&root.path().join("clones"), 0o755),
            git_config,
            search_path: vec![bin],
        })?;
        let input: StartAgentSessionMessage = serde_json::from_value(serde_json::json!({
            "protocol_version": CURRENT_PROTOCOL_VERSION, "operation_id":"restore-op", "execution_id":"restore-session",
            "payload":{"spec":{
                "node_id":"node", "agent_plugin_id":"official/agent", "agent_plugin_version":"1.0.0", "checkout_execution_id":"clone",
                "git_identity":{"name":"Fixture","email":"fixture@example.test"},
                "initial_turn":{"turn_id":"initial","content":[{"type":"text","text":"fixture"}]},
                "prior_revision":{"revision_id":"prior","final_commit":"0123456789012345678901234567890123456789","git_repository_url":"https://git.ora.test/tenant/project.git","delivery_ref":"refs/ora/deliveries/prior","base_commit":"0123456789012345678901234567890123456780"}
            }}
        }))?;
        node.database.accept_session(&input)?;
        node.database
            .start_session(&input, &NodeIncarnationId::new("original"))?;
        let journal = node.database.session_journal()?;
        let attempt = ProcessAttempt {
            execution: input.execution_id.clone(),
            host_directory: root.path().join("original-host"),
            expected_uid: 42,
            intent: HostRunIntent {
                scope: ScopeId::new(),
                run: RunId::new(),
                spec: RunSpec::new("git", root.path(), DescendantPolicy::WaitForAll),
                host_disconnect: GuardianHostDisconnect::KeepRunning,
            },
        };
        journal.record_restore_attempt(&attempt, StopContainment::BestEffort)?;
        let directory =
            node.home_directory()
                .join("revision-restores")
                .join(ora_utils::hash::sha256_hex(
                    input.execution_id.as_str().as_bytes(),
                ));
        fs::create_dir_all(&directory)?;
        fs::write(directory.join("prior.bundle"), "original process input")?;
        let transport = node.clone_config().map(|config| {
            crate::git_server::TransportRepository::location(
                &config.repository_root,
                crate::revision::RESTORE_TRANSPORT_KIND,
                &input.execution_id,
            )
        });
        if let Some(transport) = &transport {
            fs::create_dir_all(transport)?;
        }
        super::sweep_restores(&node, &journal)?;
        assert!(directory.join("prior.bundle").is_file());
        if let Some(transport) = &transport {
            assert!(transport.is_dir());
        }
        let ended = AgentSessionEnded {
            node: node.identity().clone(),
            reason: AgentSessionEndReason::Interrupted,
            detail: None,
            scheduled_result: None,
            artifact_manifest: None,
        };
        assert!(
            journal
                .end_session(&input.execution_id, ended.clone())
                .is_err()
        );
        // The process monitor alone acknowledges its independently verified original proof.
        journal.confirm_restore_stop(attempt.intent.run)?;
        journal.end_session(&input.execution_id, ended)?;
        super::sweep_restores(&node, &journal)?;
        assert!(!directory.exists());
        if let Some(transport) = &transport {
            assert!(!transport.exists());
        }
        fs::write(&directory, "unexpected local residue")?;
        super::sweep_restores(&node, &journal)?;
        assert!(directory.is_file());
        assert!(journal.session_completed(&input.execution_id)?);
        assert!(
            journal
                .pending_restore_attempts(&input.execution_id)?
                .is_empty()
        );
        Ok(())
    })
}

/// Agents run as the workload user exactly when Git does, from a directory only the Node may
/// change that overlaps none of its state; `serve` refuses every other combination before it
/// opens anything.
#[tokio::test]
async fn serve_refuses_an_agent_workload_that_does_not_match_the_process_workload() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let valid = directory(&root.join("agent"), 0o711);
    let shared = directory(&root.join("shared"), 0o777);
    let inside_home = directory(&root.join("node").join("agent"), 0o711);
    let cases = [
        config(root, Some(1000), None),
        config(root, None, Some(valid.clone())),
        config(root, Some(1000), Some(PathBuf::from("relative"))),
        config(root, Some(1000), Some(root.join("missing"))),
        config(root, Some(1000), Some(shared)),
        config(root, Some(1000), Some(inside_home)),
    ];

    let mut errors = Vec::new();
    for case in cases {
        errors.push(
            crate::serve(case, Shutdown::default())
                .await
                .expect_err("the configuration is refused")
                .to_string(),
        );
    }

    let mismatch = "agent workload_directory is required exactly when process.workload_uid is set";
    assert_eq!(
        errors,
        vec![
            mismatch.to_string(),
            mismatch.to_string(),
            "agent workload directory must be an absolute UTF-8 path without parent traversal"
                .to_string(),
            "No such file or directory (os error 2)".to_string(),
            "agent workload directory must belong to the Node identity and be writable by no one else"
                .to_string(),
            "agent workload directory overlaps Node state, host state or the clone root"
                .to_string(),
        ]
    );
}

/// The `ora-git` directory is the Node's alone: an absolute path that is missing (the Node creates
/// it) or a Node-owned directory nobody else can list or change, overlapping none of the
/// directories the Node manages; `serve` refuses anything else before it removes any socket.
#[tokio::test]
async fn serve_refuses_an_ora_git_directory_that_is_not_the_nodes_alone() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    // The tempdir itself is one of the cases: Node-owned and private, so only the overlap refuses it.
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).expect("set mode");
    let with = |ora_git: PathBuf| {
        let mut config = config(root, None, None);
        config.agent.as_mut().expect("agent").ora_git_directory = Some(ora_git);
        config
    };
    let shared = directory(&root.join("shared"), 0o755);
    let home = directory(&root.join("node"), 0o700);
    let mut workload = config(
        root,
        Some(1000),
        Some(directory(&root.join("agent"), 0o711)),
    );
    workload.agent.as_mut().expect("agent").ora_git_directory = Some(root.join("agent/ora-git"));
    let cases = [
        with(PathBuf::from("relative")),
        with(PathBuf::from("/")),
        with(shared),
        with(root.join("missing/ora-git")),
        with(root.to_path_buf()),
        with(home.join("ora-git")),
        workload,
    ];

    let mut errors = Vec::new();
    for case in cases {
        errors.push(
            super::validate(case.agent.as_ref().expect("agent"), &case)
                .expect_err("the configuration is refused")
                .to_string(),
        );
    }

    let overlap = "agent ora_git_directory overlaps Node state, host state, the clone root or the workload directory";
    assert_eq!(
        errors,
        vec![
            "agent ora_git_directory must be an absolute UTF-8 path without parent traversal, below the root",
            "agent ora_git_directory must be an absolute UTF-8 path without parent traversal, below the root",
            "agent ora_git_directory must belong to the Node identity and be readable and writable by no one else",
            "agent ora_git_directory must be created under a directory only trusted owners control",
            overlap,
            overlap,
            overlap,
        ]
    );
    for accepted in [
        root.join("ora-git"),
        directory(&root.join("sockets"), 0o711),
    ] {
        let case = with(accepted);
        super::validate(case.agent.as_ref().expect("agent"), &case).expect("accepted");
    }
}

/// Without a workload user agents share the Node identity; with one they take exactly that user,
/// with a group equal to it.
#[test]
fn the_workload_follows_the_process_workload_user() {
    let temp = tempfile::tempdir().expect("tempdir");
    let directory = temp.path().join("agent");
    let shared = config(temp.path(), None, None);
    let separate = config(temp.path(), Some(1000), Some(directory.clone()));
    let process = |config: &ServiceConfig| -> ProcessConfig { config.process.clone() };

    assert_eq!(
        (
            super::workload(shared.agent.as_ref(), &process(&shared)).expect("shared"),
            super::workload(separate.agent.as_ref(), &process(&separate)).expect("separate"),
        ),
        (
            SessionWorkload::Shared,
            SessionWorkload::Separate {
                directory,
                identity: ProcessIdentity::Linux(
                    ora_utils::process::LinuxChildIdentity::new(1000, 1000).expect("identity")
                ),
                uid: 1000,
                gid: 1000,
            },
        )
    );
}

/// An invalid restore budget is rejected before opening state, even without Agent admission.
#[tokio::test]
async fn serve_refuses_zero_restore_budget_before_opening_node_state()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let mut config = config(
        temporary.path(),
        /*workload_uid*/ None,
        /*workload_directory*/ None,
    );
    config.restore.deadline_ms = 0;
    let home = config.node.home_directory.clone();
    let error = crate::serve(config, Shutdown::default())
        .await
        .err()
        .ok_or("invalid restore configuration was accepted")?;
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!home.exists());
    Ok(())
}

/// A resumed session input, valid independently of any plugin or model provider.
fn resumed() -> ora_node_protocol::StartAgentSessionMessage {
    use ora_node_protocol::*;
    serde_json::from_value(serde_json::json!({
        "protocol_version": CURRENT_PROTOCOL_VERSION, "operation_id":"restore-op", "execution_id":"restore-session",
        "payload":{"spec":{
            "node_id":"node", "agent_plugin_id":"official/agent", "agent_plugin_version":"1.0.0", "checkout_execution_id":"clone",
            "git_identity":{"name":"Fixture","email":"fixture@example.test"},
            "initial_turn":{"turn_id":"initial","content":[{"type":"text","text":"fixture"}]},
            "prior_revision":{"revision_id":"prior","final_commit":"0123456789012345678901234567890123456789","git_repository_url":"https://git.ora.test/tenant/project.git","delivery_ref":"refs/ora/deliveries/prior","base_commit":"0123456789012345678901234567890123456780"}
        }}
    }))
    .expect("resumed session input")
}

/// The marker an Agent thread writes; only an Agent that started produces one.
fn thread_event() -> ora_node_protocol::ThreadEvent {
    ora_node_protocol::ThreadEvent {
        turn_id: None,
        record: serde_json::Map::new(),
        truncated: false,
        visible_text: None,
        visible_text_truncated: false,
        history_sequence: None,
    }
}

/// Rebuilds the deployment's Node configuration, which opening a Node consumes.
fn node_config(deployment: &ServiceConfig) -> crate::NodeConfig {
    serde_json::from_value(
        serde_json::to_value(&deployment.node).expect("node configuration encodes"),
    )
    .expect("node configuration decodes")
}

/// A deployment whose clone, Git Server and host sides all point inside `root`.
fn restored_deployment(
    root: &Path,
) -> Result<(ServiceConfig, crate::CloneConfig), Box<dyn std::error::Error>> {
    use ora_node_db::NodeIdentity;
    use ora_node_protocol::NodeId;
    let mut deployment = config(
        root, /*workload_uid*/ None, /*workload_directory*/ None,
    );
    deployment.node.identity = NodeIdentity::Require(NodeId::new("node"));
    let bin = directory(&root.join("bin"), 0o755);
    let git_config = root.join("gitconfig");
    fs::write(&git_config, "")?;
    let clone = crate::CloneConfig {
        repository_root: directory(&root.join("clones"), 0o755),
        git_config,
        search_path: vec![bin],
    };
    Ok((deployment, clone))
}

/// The Controller deletes every event it acknowledges from the Node's outbox. A session whose
/// Agent had already produced one had finished restoring before it started, so startup recovery
/// ends it as interrupted instead of fencing the whole Node on missing evidence.
#[test]
fn startup_recovery_ends_a_restore_proven_by_the_sequence_the_controller_acked()
-> Result<(), Box<dyn std::error::Error>> {
    use ora_node_protocol::*;
    ora_logging::initialize_test_clock();
    let root = tempfile::tempdir()?;
    let (deployment, _) = restored_deployment(root.path())?;
    let home = deployment.node.home_directory.clone();
    let process = deployment.process.clone();
    let input = resumed();
    {
        let mut node = crate::Node::open(
            node_config(&deployment),
            process.clone(),
            Shutdown::default(),
        )?;
        node.database.accept_session(&input)?;
        assert!(
            node.database
                .start_session(&input, &NodeIncarnationId::new("original"))?
        );
        let journal = node.database.session_journal()?;
        let sequence = journal.append_thread_event(&input.execution_id, thread_event())?;
        drop(journal);
        // The production path: an online Controller acknowledges the event it consumed.
        node.database.acknowledge(&EventAckMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: input.operation_id.clone(),
            execution_id: input.execution_id.clone(),
            sequence,
            payload: EventAck {
                node_id: NodeId::new("node"),
            },
        })?;
        assert!(node.database.pending_events()?.is_empty());
    }
    // A Node still on v10 opens the file and rebuilds the restore ledger from real history.
    // The downgrade also restores the v10 identity check and terminal-event outbox, which v12
    // widened; the schema check compares their exact SQL text.
    let revision_schema = include_str!("../../../../../crates/node-db/src/revision.sql");
    let identities = revision_schema
        .split("CREATE TABLE execution_identities")
        .nth(1)
        .ok_or("missing v9 identity schema")?
        .split("INSERT INTO execution_identities")
        .next()
        .ok_or("missing v9 identity columns")?;
    let outbox = revision_schema
        .split("CREATE TABLE revision_outbox")
        .nth(1)
        .ok_or("missing v9 outbox schema")?
        .split("CREATE TRIGGER")
        .next()
        .ok_or("missing v9 outbox columns")?;
    let connection = rusqlite::Connection::open(home.join("ora-node.sqlite3"))?;
    connection.execute_batch(&format!(
        "PRAGMA foreign_keys=OFF;
        DROP TABLE IF EXISTS session_history_admissions;
        DROP TRIGGER bind_new_scheduled_artifact; DROP TABLE scheduled_artifact_deliveries;
        DROP TABLE restore_process_attempts; DROP TABLE session_restores;
        CREATE TEMP TABLE v10_ids AS SELECT * FROM execution_identities;
        DROP TABLE execution_identities;
        CREATE TABLE execution_identities{identities}
        INSERT INTO execution_identities SELECT * FROM v10_ids; DROP TABLE v10_ids;
        CREATE TEMP TABLE v10_events AS SELECT * FROM revision_outbox;
        DROP TABLE revision_outbox;
        CREATE TABLE revision_outbox{outbox}
        INSERT INTO revision_outbox SELECT * FROM v10_events; DROP TABLE v10_events;
        PRAGMA user_version=10;"
    ))?;
    drop(connection);
    let mut node = crate::Node::open(
        node_config(&deployment),
        process.clone(),
        Shutdown::default(),
    )?;
    let journal = node.database.session_journal()?;
    // The Agent started after v10's synchronous restore, so no restore responsibility remains.
    assert_eq!(journal.restore_phase(&input.execution_id)?, None);
    let stops = crate::managed::RestoreStops::new(
        journal.clone(),
        process,
        Duration::from_secs(/*secs*/ 60),
    );
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(super::reconcile(
            &mut node,
            None,
            &stops,
            crate::managed::StopCheck::Handover,
            None,
        ))?;
    assert!(journal.session_completed(&input.execution_id)?);
    assert!(node.database.unfinished_runtime_executions()?.is_empty());
    assert!(matches!(
        node.database.pending_events()?.as_slice(),
        [NodeToControllerMessage::AgentSessionEnded(message)]
            if matches!(
                &message.payload,
                AgentSessionResult::AgentSessionEnded(ended)
                    if ended.reason == AgentSessionEndReason::Interrupted
            )
    ));
    Ok(())
}

/// The prior Revision a resumed session restores.
fn prior_revision() -> ora_node_protocol::PriorRevision {
    use ora_node_protocol::*;
    PriorRevision {
        revision_id: RevisionId::new("prior"),
        final_commit: CommitId::new("0".repeat(40)),
        git_repository_url: GitRepositoryUrl::parse("https://git.ora.test/tenant/project.git")
            .expect("valid repository URL"),
        delivery_ref: DeliveryRef::parse("refs/ora/deliveries/prior").expect("valid delivery ref"),
        base_commit: CommitId::new("0".repeat(40)),
    }
}

/// A Run the host still reports as running with its cleanup outstanding.
fn running(
    scope: ora_process_protocol::ScopeId,
    run: ora_process_protocol::RunId,
) -> ora_process_protocol::HostRunView {
    use ora_process_protocol::*;
    HostRunView {
        scope,
        run,
        stop_requested: false,
        last_observed: Some(RunSnapshot {
            id: run,
            launch: LaunchFact::Started,
            direct: DirectProcessState::Running,
            cleanup: CleanupState::Pending,
        }),
        coordination: HostCoordination::Observing,
    }
}

/// Answers like a host whose guardian is unreachable: the Run never ends and no Close is ever
/// confirmed, so no original stop can be proven.
fn serve_refusing_close(
    listener: std::os::unix::net::UnixListener,
    starts: &std::sync::atomic::AtomicUsize,
    stop: &std::sync::atomic::AtomicBool,
) {
    use ora_process_protocol::*;
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let mut scope = ScopeId::new();
    while !stop.load(Ordering::SeqCst) {
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(/*millis*/ 5));
                continue;
            }
            Err(_) => return,
        };
        stream.set_nonblocking(false).expect("blocking connection");
        loop {
            let mut length = [0u8; 4];
            match stream.read_exact(&mut length) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                    ) =>
                {
                    break;
                }
                Err(_) => break,
            }
            let mut bytes = vec![0; u32::from_be_bytes(length) as usize];
            if stream.read_exact(&mut bytes).is_err() {
                break;
            }
            let Ok(request) = decode_guardian_payload::<HostRequest>(&bytes) else {
                break;
            };
            let reply = match request.operation {
                HostOperation::CreateScope { scope: requested } => {
                    scope = requested;
                    HostReply::Scope(HostScopeView {
                        scope,
                        close_requested: false,
                        last_observed: Some(ScopeState::Open),
                        coordination: HostCoordination::Observing,
                    })
                }
                HostOperation::Start { intent } => {
                    starts.fetch_add(1, Ordering::SeqCst);
                    scope = intent.scope;
                    HostReply::Run(running(scope, intent.run))
                }
                HostOperation::QueryRun { run } => HostReply::Run(running(scope, run)),
                HostOperation::Close { scope: requested } => {
                    scope = requested;
                    HostReply::Rejected(HostRejection::GuardianUnavailable)
                }
                _ => HostReply::Rejected(HostRejection::IntentRejected),
            };
            let Ok(frame) = encode_guardian_frame(&reply) else {
                break;
            };
            if stream.write_all(&frame).is_err() {
                break;
            }
        }
    }
}

/// A restore command the Node runs as itself records its original Run before the host sees it.
/// When that Run's Close stays unconfirmed the session reaches no terminal state, and the
/// transport repository the original Git may still read stays on disk.
#[test]
fn unconfirmed_node_identity_restore_run_reaches_no_terminal_state()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::session::{PriorRevisionRestore, RestoreCancel, RestoreFailure, RestoreRequest};
    use ora_node_protocol::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    ora_logging::initialize_test_clock();
    let root = tempfile::tempdir()?;
    let (mut deployment, clone) = restored_deployment(root.path())?;
    let host_directory = directory(&root.path().join("fake-host"), 0o700);
    deployment.process.host_directory = host_directory.clone();
    // SAFETY: reading this test's own effective user changes nothing.
    deployment.process.expected_uid = unsafe { libc::geteuid() };
    let checkout = directory(&root.path().join("checkout"), 0o755);
    fs::create_dir(checkout.join(".git"))?;
    let starts = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let listener = std::os::unix::net::UnixListener::bind(host_directory.join("host.sock"))?;
    let host = {
        let starts = Arc::clone(&starts);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || serve_refusing_close(listener, &starts, &stop))
    };
    let mut node = crate::Node::open(
        deployment.node,
        deployment.process.clone(),
        Shutdown::default(),
    )?;
    node.configure_clone(clone)?;
    node.configure_git_access(crate::GitAccessConfig {
        ca_cert: root.path().join("ca.pem"),
        client_cert: root.path().join("client.pem"),
        client_key: root.path().join("client-key.pem"),
    });
    let input = resumed();
    node.database.accept_session(&input)?;
    assert!(
        node.database
            .start_session(&input, &NodeIncarnationId::new("original"))?
    );
    let journal = node.database.session_journal()?;
    let stops = crate::managed::RestoreStops::new(
        journal.clone(),
        deployment.process,
        Duration::from_millis(/*millis*/ 300),
    );
    let restorer = super::restorer(
        &node,
        deployment.agent.as_ref(),
        Shutdown::default(),
        crate::revision::RestorePolicy {
            deadline: Duration::from_millis(/*millis*/ 100),
            stop_confirmation: Duration::from_millis(/*millis*/ 300),
        },
        &stops,
    )?
    .ok_or("the deployment composes a restore")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let restored = runtime.block_on(restorer.restore(
        RestoreRequest {
            operation: input.operation_id.clone(),
            execution: input.execution_id.clone(),
            checkout: checkout.clone(),
            branch: "main".to_owned(),
            base_commit: CommitId::new("0".repeat(40)),
            prior: prior_revision(),
        },
        RestoreCancel::default(),
    ));
    assert_eq!(restored.err(), Some(RestoreFailure::Unavailable));
    // The Node-identity command was journaled and started; the workload's never ran.
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        journal.pending_restore_attempts(&input.execution_id)?.len(),
        1
    );
    let ended = AgentSessionEnded {
        node: node.identity().clone(),
        reason: AgentSessionEndReason::AgentFailed,
        detail: Some("prior_revision_unavailable".into()),
        scheduled_result: None,
        artifact_manifest: None,
    };
    journal.defer_restore_end(&input.execution_id, &ended)?;
    assert!(!runtime.block_on(restorer.confirm_stop(&input.execution_id)));
    let unconfirmed = journal.pending_restore_attempts(&input.execution_id)?;
    assert_eq!(unconfirmed.len(), 1);
    assert!(unconfirmed[0].stop_unconfirmed);
    assert!(matches!(
        journal.end_session(&input.execution_id, ended),
        Err(ora_node_db::Error::ResourceConflict)
    ));
    assert!(node.database.pending_events()?.is_empty());
    // The original command may still read the objects it was given.
    let transport = crate::git_server::TransportRepository::location(
        &node
            .clone_config()
            .ok_or("configured clone")?
            .repository_root,
        crate::revision::RESTORE_TRANSPORT_KIND,
        &input.execution_id,
    );
    assert!(transport.is_dir());
    stop.store(true, Ordering::SeqCst);
    host.join().ok();
    Ok(())
}

/// Startup cleanup keeps the transport repository an unconfirmed original restore command may
/// still be reading, and the same real path releases it once the session's stop is confirmed.
#[test]
fn startup_cleanup_keeps_restore_inputs_until_original_stop_is_confirmed()
-> Result<(), Box<dyn std::error::Error>> {
    use ora_node_db::ProcessAttempt;
    use ora_node_protocol::*;
    use ora_process_protocol::*;
    ora_logging::initialize_test_clock();
    let root = tempfile::tempdir()?;
    let (deployment, clone) = restored_deployment(root.path())?;
    let mut node = crate::Node::open(deployment.node, deployment.process, Shutdown::default())?;
    let input = resumed();
    node.database.accept_session(&input)?;
    assert!(
        node.database
            .start_session(&input, &NodeIncarnationId::new("original"))?
    );
    let journal = node.database.session_journal()?;
    let attempt = ProcessAttempt {
        execution: input.execution_id.clone(),
        host_directory: root.path().join("original-host"),
        expected_uid: 42,
        intent: HostRunIntent {
            scope: ScopeId::new(),
            run: RunId::new(),
            spec: RunSpec::new("git", root.path(), DescendantPolicy::WaitForAll),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        },
    };
    journal.record_restore_attempt(&attempt, StopContainment::BestEffort)?;
    // The deployment canonicalizes its roots, so the restarted Node uses the same directory.
    let repository_root = std::fs::canonicalize(&clone.repository_root)?;
    let transport = crate::git_server::TransportRepository::location(
        &repository_root,
        crate::revision::RESTORE_TRANSPORT_KIND,
        &input.execution_id,
    );
    fs::create_dir_all(&transport)?;
    // A repository of no recorded session is still cleared by the same startup pass.
    let stale = repository_root
        .join(".ora-transport")
        .join("restore-stale-attempt");
    fs::create_dir_all(&stale)?;
    node.configure_clone(clone.clone())?;
    assert_eq!(
        (transport.is_dir(), stale.exists()),
        (true, false),
        "startup cleanup must keep only inputs an original command may still read"
    );
    journal.confirm_restore_stop(attempt.intent.run)?;
    journal.end_session(
        &input.execution_id,
        AgentSessionEnded {
            node: node.identity().clone(),
            reason: AgentSessionEndReason::Interrupted,
            detail: None,
            scheduled_result: None,
            artifact_manifest: None,
        },
    )?;
    node.configure_clone(clone)?;
    assert!(!transport.exists());
    Ok(())
}
