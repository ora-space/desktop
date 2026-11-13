//! Real commands holding real attempt locks, with the production stop monitor and durable
//! session ledger.
use super::exec_tests::Fixture;
use super::*;
use crate::revision::RestoreGitRunner;
use ora_node_db::{NodeDatabase, NodeIdentity, SessionJournal};
use ora_node_protocol::*;
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, TokioProcessSpawner};
use pretty_assertions::assert_eq;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Scoped TRACE keeps shared production callsites observable in every test ordering.
pub(super) fn with_trace<T>(action: impl FnOnce() -> T) -> T {
    let dispatcher = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    );
    tracing::dispatcher::with_default(&dispatcher, action)
}

/// A valid resumed session, independent of any plugin or model provider.
pub(super) fn session() -> StartAgentSessionMessage {
    StartAgentSessionMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: OperationId::new("restore-op"),
        execution_id: ExecutionId::new("restore-session"),
        payload: StartAgentSession {
            spec: AgentSessionSpec {
                workspace_directory: Vec::new(),
                agent_configuration: None,
                node_id: NodeId::new("node"),
                agent_plugin_id: PluginId::new("official/agent"),
                agent_plugin_version: PluginVersion::new("1.0.0"),
                checkout_execution_id: ExecutionId::new("clone"),
                model_binding_id: None,
                git_identity: GitIdentity {
                    name: "Fixture".into(),
                    email: "fixture@example.test".into(),
                },
                initial_turn: UserTurn {
                    turn_id: TurnId::new("initial"),
                    content: vec![ContentBlock::Text {
                        text: "fixture".into(),
                    }],
                },
                prior_revision: Some(PriorRevision {
                    revision_id: RevisionId::new("prior"),
                    final_commit: CommitId::new("0".repeat(40)),
                    git_repository_url: GitRepositoryUrl::parse(
                        "https://git.ora.test/tenant/project.git",
                    )
                    .expect("valid repository URL"),
                    delivery_ref: DeliveryRef::parse("refs/ora/deliveries/prior")
                        .expect("valid delivery ref"),
                    base_commit: CommitId::new("0".repeat(40)),
                }),
                trigger: None,
            },
        },
    }
}

/// The terminal fact a restore defers while its stop evidence is missing.
fn ended() -> AgentSessionEnded {
    AgentSessionEnded {
        node: NodeRuntimeIdentity {
            node_id: NodeId::new("node"),
            incarnation_id: NodeIncarnationId::new("original"),
        },
        reason: AgentSessionEndReason::AgentFailed,
        detail: Some("prior_revision_unavailable".into()),
        scheduled_result: None,
        artifact_manifest: None,
    }
}

/// A started resumed session in a fresh Node database under the fixture.
fn started(
    fixture: &Fixture,
) -> Result<(NodeDatabase, SessionJournal), Box<dyn std::error::Error>> {
    let input = session();
    let mut db = NodeDatabase::open(
        &fixture.directory.path().join("node.sqlite"),
        NodeIdentity::Require(NodeId::new("node")),
    )?;
    db.accept_session(&input)?;
    db.start_session(&input, &NodeIncarnationId::new("original"))?;
    let journal = db.session_journal()?;
    Ok((db, journal))
}

/// Records a restore attempt the way a dispatch does: lock file first, then the record.
fn recorded(
    fixture: &Fixture,
    journal: &SessionJournal,
) -> Result<ora_node_db::ProcessAttempt, Box<dyn std::error::Error>> {
    let attempt = ora_node_db::ProcessAttempt {
        execution: session().execution_id,
        id: AttemptId::generate(),
    };
    fixture.locks.create(
        &attempt.id,
        fixture.directory.path(),
        LockDurability::Recorded,
    )?;
    journal.record_restore_attempt(
        &attempt,
        &AttemptCommand {
            cwd: fixture.directory.path().to_path_buf(),
            args: vec!["fetch".into()],
        },
        StopContainment::BestEffort,
    )?;
    Ok(attempt)
}

/// Unknown evidence cannot authorize a terminal event, and the stop bound only ends the waiting:
/// the original attempt stays pending until its own lock proves it stopped, which then settles
/// the session exactly once. A lock file that is missing never proves anything.
#[test]
fn unknown_stop_expires_without_terminal_then_original_evidence_settles_once() -> TestResult {
    with_trace(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                ora_logging::initialize_test_clock();
                for lock_file in [LockFile::Held, LockFile::Missing] {
                    let fixture = Fixture::new()?;
                    let input = session();
                    let (db, journal) = started(&fixture)?;
                    let attempt = recorded(&fixture, &journal)?;
                    // An open file that took the lock stands for any process of the original
                    // tree that the Node cannot end: the evidence only asks whether one exists.
                    let holder = match lock_file {
                        LockFile::Held => Some(fixture.locks.hold(&attempt.id)?),
                        LockFile::Missing => {
                            fixture.locks.retire(&attempt.id);
                            None
                        }
                    };
                    journal.defer_restore_end(&input.execution_id, &ended())?;
                    let mut config = fixture.config();
                    config.cleanup_timeout_ms = 40;
                    let stops = RestoreStops::new(
                        journal.clone(),
                        config,
                        Duration::from_secs(/*secs*/ 2),
                        fixture.locks.clone(),
                    );
                    assert!(
                        !stops
                            .poll(&input.execution_id, StopCheck::WithinBound)
                            .await?
                    );
                    assert!(journal.end_session(&input.execution_id, ended()).is_err());
                    assert!(db.pending_events()?.is_empty());
                    assert!(
                        !tokio::time::timeout(
                            Duration::from_secs(/*secs*/ 3),
                            stops.wait(&input.execution_id)
                        )
                        .await?
                    );
                    let before = journal.pending_restore_attempts(&input.execution_id)?;
                    assert_eq!(
                        before
                            .iter()
                            .map(|pending| (pending.process.clone(), pending.stop_unconfirmed))
                            .collect::<Vec<_>>(),
                        vec![(attempt.clone(), true)]
                    );
                    assert!(
                        !stops
                            .poll(&input.execution_id, StopCheck::WithinBound)
                            .await?
                    );
                    assert_eq!(
                        journal.pending_restore_attempts(&input.execution_id)?,
                        before
                    );
                    // Only the original attempt's own lock supplies late evidence.
                    let Some(holder) = holder else {
                        assert!(!stops.poll(&input.execution_id, StopCheck::Handover).await?);
                        assert!(journal.end_session(&input.execution_id, ended()).is_err());
                        continue;
                    };
                    drop(holder);
                    assert!(stops.poll(&input.execution_id, StopCheck::Handover).await?);
                    journal.end_session(
                        &input.execution_id,
                        journal
                            .deferred_restore_end(&input.execution_id)?
                            .ok_or("missing deferred end")?,
                    )?;
                    assert!(journal.end_session(&input.execution_id, ended()).is_err());
                    assert_eq!(
                        (db.pending_events()?.len(), fixture.locks.entries()?),
                        (1, vec![])
                    );
                }
                Ok(())
            })
    })
}

/// Which lock file an unsettled restore attempt has.
enum LockFile {
    /// Present and held by something the Node cannot end.
    Held,
    /// Gone, as when a record outlived its file: evidence is missing, not positive.
    Missing,
}

/// Real Git is cut at the restore's cutoff although its own network budget is longer. The stop
/// clock is anchored before the kill, and the command only returns once its lock confirmed the
/// stop: nothing stays pending and the lock file is retired.
#[test]
fn restore_cutoff_ends_real_git_and_confirms_its_stop() -> TestResult {
    with_trace(|| {
        ora_logging::initialize_test_clock();
        let fixture = Fixture::new()?;
        let input = session();
        let (_db, journal) = started(&fixture)?;
        let stops = RestoreStops::new(
            journal.clone(),
            fixture.config(),
            Duration::from_secs(/*secs*/ 30),
            fixture.locks.clone(),
        );
        let mut runner = fixture.git(fixture.config())?;
        runner.track_restore(stops);
        runner.bind_restore(
            &input.execution_id,
            crate::session::RestoreCancel::default(),
        );
        let command = fixture.script(
            "printf started > started; exec sleep 60",
            GitIntent::Network,
        );

        let begun = Instant::now();
        let result = runner.run_bounded_before(
            &command,
            /*stdout*/ 1024,
            /*stderr*/ 1024,
            begun + Duration::from_millis(/*millis*/ 500),
        );

        assert!(
            matches!(result, Err(GitExecError::OutputReadFailed { source, .. })
            if source.kind() == std::io::ErrorKind::TimedOut)
        );
        assert_eq!(
            (
                fixture.directory.path().join("started").is_file(),
                begun.elapsed() < Duration::from_secs(/*secs*/ 10),
                journal.pending_restore_attempts(&input.execution_id)?,
                journal.restore_stop_requested(&input.execution_id)?,
                fixture.locks.entries()?,
            ),
            (true, true, vec![], false, vec![])
        );
        Ok(())
    })
}

/// A Node that was killed left its restore command running and only a clue behind. The
/// restarted Node ends the group the clue names and confirms the stop of the original attempt;
/// it never starts a replacement.
#[test]
fn a_restarted_node_ends_and_confirms_the_original_restore_command() -> TestResult {
    with_trace(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                ora_logging::initialize_test_clock();
                let fixture = Fixture::new()?;
                let input = session();
                let (_db, journal) = started(&fixture)?;
                let attempt = recorded(&fixture, &journal)?;
                let original = TokioProcessSpawner::new().spawn(
                    ProcessSpec::new("/bin/sh")
                        .args(["-c", "sleep 60 & exec sleep 60"])
                        .keep_alive_on_drop()
                        .inherit_descriptor(fixture.locks.hold(&attempt.id)?),
                )?;
                fixture
                    .locks
                    .note_child(&attempt.id, original.id().ok_or("pid")?);
                // The killed Node's handle is gone; only the lock file and the clue remain.
                drop(original);
                let stops = RestoreStops::new(
                    journal.clone(),
                    fixture.config(),
                    Duration::from_secs(/*secs*/ 30),
                    fixture.locks.clone(),
                );

                let confirmed = stops.poll(&input.execution_id, StopCheck::Handover).await?;

                assert_eq!(
                    (
                        confirmed,
                        journal.pending_restore_attempts(&input.execution_id)?,
                        fixture.locks.entries()?,
                    ),
                    (true, vec![], vec![])
                );
                Ok(())
            })
    })
}
