//! Durable stop clocks and checkout responsibility across real SQLite reopen and migration.
use super::*;
use ora_process_protocol::*;
use pretty_assertions::assert_eq;

/// Supplies an immutable prior input without relying on a running Git fixture.
fn resumed() -> StartAgentSessionMessage {
    let mut input = command();
    input.payload.spec.prior_revision = Some(PriorRevision {
        revision_id: RevisionId::new("prior"),
        final_commit: CommitId::new("0123456789012345678901234567890123456789"),
        git_repository_url: GitRepositoryUrl::parse("https://git.ora.test/tenant/project.git")
            .unwrap(),
        delivery_ref: DeliveryRef::parse("refs/ora/deliveries/prior").unwrap(),
        base_commit: CommitId::new("0123456789012345678901234567890123456780"),
    });
    input
}

/// No process can exist until this exact association commits.
fn attempt(input: &StartAgentSessionMessage, directory: &std::path::Path) -> ProcessAttempt {
    ProcessAttempt {
        execution: input.execution_id.clone(),
        host_directory: directory.join("original-host"),
        expected_uid: 42,
        intent: HostRunIntent {
            scope: ScopeId::new(),
            run: RunId::new(),
            spec: RunSpec::new("git", directory, DescendantPolicy::WaitForAll),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        },
    }
}

/// Rewinds the fixture to the v10 schema so the v11 backfill runs against real history.
fn reopen_after_v10_downgrade(
    path: &std::path::Path,
) -> Result<NodeDatabase, Box<dyn std::error::Error>> {
    // v12 also changed the shared identity check and the terminal-event outbox, so both are rebuilt
    // to their exact v10 definitions (the schema check compares SQL text).
    let revision = include_str!("../../revision.sql");
    let identities = revision
        .split("CREATE TABLE execution_identities")
        .nth(1)
        .ok_or("missing v9 identity schema")?
        .split("INSERT INTO execution_identities")
        .next()
        .ok_or("missing v9 identity columns")?;
    let outbox = revision
        .split("CREATE TABLE revision_outbox")
        .nth(1)
        .ok_or("missing v9 outbox schema")?
        .split("CREATE TRIGGER")
        .next()
        .ok_or("missing v9 outbox columns")?;
    Connection::open(path)?.execute_batch(&format!(
        "PRAGMA foreign_keys=OFF;
        DROP TRIGGER refuse_fenced_execution; DROP TABLE execution_non_acceptance;
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
    Ok(NodeDatabase::open(path, NodeIdentity::Discover)?)
}

/// Expiry, restart and changed configuration preserve the original clock, identity and fence.
#[test]
fn unknown_stop_retains_clock_terminal_and_checkout_responsibility()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node")))?;
    let input = resumed();
    db.accept_session(&input)?;
    assert!(db.start_session(&input, &NodeIncarnationId::new("original"))?);
    let journal = db.session_journal()?;
    let process = attempt(&input, directory.path());
    journal.record_restore_attempt(&process, StopContainment::BestEffort)?;
    journal.request_session_restore_stop(&input.execution_id, 1000, 600_000)?;
    journal.request_restore_stop(process.intent.run, 20_000, 1_000_000)?;
    journal.defer_restore_end(&input.execution_id, &ended())?;
    journal.mark_restore_stop_unconfirmed(process.intent.run, 601_000)?;
    let expected = vec![RestoreAttempt {
        process: process.clone(),
        guarantee: StopContainment::BestEffort,
        first_stop_at_ms: Some(1000),
        stop_bound_at_ms: Some(601_000),
        stop_unconfirmed: true,
    }];
    assert_eq!(
        journal.pending_restore_attempts(&input.execution_id)?,
        expected
    );
    assert!(matches!(
        journal.end_session(&input.execution_id, ended()),
        Err(Error::ResourceConflict)
    ));
    assert!(db.pending_events()?.is_empty());
    let mut replacement = input.clone();
    replacement.execution_id = ExecutionId::new("replacement");
    replacement.operation_id = OperationId::new("replacement-op");
    assert!(matches!(
        db.accept_session(&replacement),
        Err(Error::ResourceConflict)
    ));
    assert!(
        journal
            .record_restore_attempt(&attempt(&input, directory.path()), StopContainment::Strong)
            .is_err()
    );
    assert!(journal.restored(&input.execution_id).is_err());
    assert!(journal.agent_admitted(&input.execution_id).is_err());
    drop(journal);
    drop(db);
    let db = NodeDatabase::open(&path, NodeIdentity::Discover)?;
    let journal = db.session_journal()?;
    journal.request_session_restore_stop(&input.execution_id, 900_000, 2_000_000)?;
    assert_eq!(
        journal.pending_restore_attempts(&input.execution_id)?,
        expected
    );
    assert_eq!(
        journal.deferred_restore_end(&input.execution_id)?,
        Some(ended())
    );
    // Only original host proof authorizes the caller to acknowledge this association.
    journal.confirm_restore_stop(process.intent.run)?;
    journal.end_session(
        &input.execution_id,
        journal
            .deferred_restore_end(&input.execution_id)?
            .ok_or("missing deferred end")?,
    )?;
    assert!(journal.end_session(&input.execution_id, ended()).is_err());
    assert_eq!(db.pending_events()?.len(), 1);
    Ok(())
}

/// A v10 session that was accepted but never started has no restore process, so the upgrade keeps
/// its pending phase and startup recovery may end it as interrupted without stop evidence.
#[test]
fn upgrade_keeps_an_accepted_restore_pending() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node")))?;
    let input = resumed();
    db.accept_session(&input)?;
    drop(db);
    let mut db = reopen_after_v10_downgrade(&path)?;
    let journal = db.session_journal()?;
    // Accepted never started an actor, so no restore process can exist for it.
    assert_eq!(
        journal.restore_phase(&input.execution_id)?,
        Some(RestorePhase::Pending)
    );
    journal.end_session(&input.execution_id, ended())?;
    assert!(db.unfinished_runtime_executions()?.is_empty());
    let scope = super::super::repository::runtime_scope(db.node_id());
    assert_eq!(db.bind_runtime(&scope)?, Vec::<String>::new());
    Ok(())
}

/// Missing stop origins are fixed on first recovery, never renewed on later handovers.
#[test]
fn missing_stop_origin_is_anchored_only_once() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut db = NodeDatabase::open(
        &directory.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )?;
    let input = resumed();
    db.accept_session(&input)?;
    db.start_session(&input, &NodeIncarnationId::new("original"))?;
    let journal = db.session_journal()?;
    let process = attempt(&input, directory.path());
    journal.record_restore_attempt(&process, StopContainment::Strong)?;
    journal.request_restore_stop(process.intent.run, 50, 100)?;
    journal.request_restore_stop(process.intent.run, 500, 1000)?;
    assert_eq!(
        journal.pending_restore_attempts(&input.execution_id)?,
        vec![RestoreAttempt {
            process,
            guarantee: StopContainment::Strong,
            first_stop_at_ms: Some(50),
            stop_bound_at_ms: Some(150),
            stop_unconfirmed: false
        }]
    );
    Ok(())
}

/// A completed pre-journal restore stays history and must not fence later admission.
#[test]
fn upgrade_keeps_completed_legacy_restore_out_of_admission_fences()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node")))?;
    let input = resumed();
    db.accept_session(&input)?;
    db.session_journal()?
        .end_session(&input.execution_id, ended())?;
    let original = db.pending_events()?;
    drop(db);
    let mut db = reopen_after_v10_downgrade(&path)?;
    let journal = db.session_journal()?;
    assert_eq!(journal.restore_phase(&input.execution_id)?, None);
    assert_eq!(db.pending_events()?, original);
    assert!(db.unfinished_runtime_executions()?.is_empty());
    drop(journal);
    // Runtime admission is the first gate Cloud passes and must stay open.
    let scope = super::super::repository::runtime_scope(db.node_id());
    assert_eq!(db.bind_runtime(&scope)?, Vec::<String>::new());
    // The same checkout stays reusable by the next controlled session.
    let mut replacement = input;
    replacement.operation_id = OperationId::new("replacement-op");
    replacement.execution_id = ExecutionId::new("replacement-execution");
    let mut permit = scope;
    permit.execution_id = replacement.execution_id.as_str().into();
    permit.node_operation_id = replacement.operation_id.as_str().into();
    db.accept_controlled_session(&replacement, &permit)?;
    Ok(())
}

/// An in-flight pre-journal restore has no stop proof, so both admission fences stay.
#[test]
fn upgrade_keeps_inflight_legacy_restore_fenced() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node")))?;
    let input = resumed();
    db.accept_session(&input)?;
    assert!(db.start_session(&input, &NodeIncarnationId::new("original"))?);
    drop(db);
    let mut db = reopen_after_v10_downgrade(&path)?;
    let journal = db.session_journal()?;
    assert_eq!(
        journal.restore_phase(&input.execution_id)?,
        Some(RestorePhase::LegacyUnknown)
    );
    drop(journal);
    let mut replacement = input;
    replacement.operation_id = OperationId::new("replacement-op");
    replacement.execution_id = ExecutionId::new("replacement-execution");
    // The checkout cannot be reused while the original stop is unknown.
    assert!(matches!(
        db.accept_session(&replacement),
        Err(Error::ResourceConflict)
    ));
    // Runtime admission reads the same ledger and refuses an open Cloud binding.
    let scope = super::super::repository::runtime_scope(db.node_id());
    assert!(matches!(
        db.bind_runtime(&scope),
        Err(Error::ResourceConflict)
    ));
    Ok(())
}

/// An in-flight v10 restore whose Agent produced thread events had finished restoring. The
/// Controller's acknowledgements delete those events, so the upgrade must read the sequence the
/// database keeps instead and release both admission fences.
#[test]
fn upgrade_releases_inflight_restore_proven_after_events_were_acknowledged()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("db");
    let mut db = NodeDatabase::open(&path, NodeIdentity::Require(NodeId::new("node")))?;
    let input = resumed();
    db.accept_session(&input)?;
    let journal = db.session_journal()?;
    assert!(db.start_session(&input, &NodeIncarnationId::new("original"))?);
    let sequence = journal.append_thread_event(&input.execution_id, event())?;
    drop(journal);
    // The production path: an online Controller acknowledges the event it consumed.
    db.acknowledge(&ack(sequence.value()))?;
    assert!(db.pending_events()?.is_empty());
    drop(db);
    let mut db = reopen_after_v10_downgrade(&path)?;
    let journal = db.session_journal()?;
    // The Agent started after v10's synchronous restore, so no restore responsibility remains.
    assert_eq!(journal.restore_phase(&input.execution_id)?, None);
    // Startup recovery can end the session as interrupted, which releases both fences.
    journal.end_session(&input.execution_id, ended())?;
    assert!(db.unfinished_runtime_executions()?.is_empty());
    let scope = super::super::repository::runtime_scope(db.node_id());
    assert_eq!(db.bind_runtime(&scope)?, Vec::<String>::new());
    let mut replacement = input;
    replacement.operation_id = OperationId::new("replacement-op");
    replacement.execution_id = ExecutionId::new("replacement-execution");
    let mut permit = scope;
    permit.execution_id = replacement.execution_id.as_str().into();
    permit.node_operation_id = replacement.operation_id.as_str().into();
    db.accept_controlled_session(&replacement, &permit)?;
    Ok(())
}
