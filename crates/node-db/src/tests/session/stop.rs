//! Stop ownership, root audit, retained terminal intent and pre-migration drain use real SQLite.
use super::*;
use pretty_assertions::assert_eq;

/// One original Agent generation, without runtime settings or absolute package paths.
fn identity(input: &StartAgentSessionMessage) -> SessionStopIdentity {
    SessionStopIdentity {
        attempt: AttemptId::generate(),
        execution: input.execution_id.clone(),
        checkout: input.payload.spec.checkout_execution_id.clone(),
        node: NodeRuntimeIdentity {
            node_id: input.payload.spec.node_id.clone(),
            incarnation_id: NodeIncarnationId::new("original"),
        },
        host: SessionStopHost {
            boot_id: "boot-before".into(),
            trusted_host_id: Some("physical-host".into()),
        },
        kind: SessionStopKind::Agent,
        packages: vec![SessionStopPackage {
            plugin_id: input.payload.spec.agent_plugin_id.clone(),
            plugin_version: input.payload.spec.agent_plugin_version.clone(),
            view_key: None,
        }],
    }
}

/// Opening and starting a session gives the stop journal a real Running execution to protect.
fn running(path: &std::path::Path) -> (NodeDatabase, SessionJournal, StartAgentSessionMessage) {
    let mut db = NodeDatabase::open(path, NodeIdentity::Require(NodeId::new("node"))).unwrap();
    let input = command();
    db.accept_session(&input).unwrap();
    db.start_session(&input, &NodeIncarnationId::new("original"))
        .unwrap();
    let journal = db.session_journal().unwrap();
    (db, journal, input)
}

#[test]
fn restart_preserves_clock_identity_terminal_intent_and_resource_fence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (db, journal, input) = running(&path);
    let original = identity(&input);
    journal.record_session_stop(&original, 10).unwrap();
    journal.record_session_stop(&original, 11).unwrap();
    let mut changed = original.clone();
    changed.host.boot_id = "other".into();
    assert!(journal.record_session_stop(&changed, 12).is_err());
    journal
        .change_session_stop(
            &original.attempt,
            SessionStopChange::Request {
                at_ms: 20,
                bound_at_ms: 620,
            },
        )
        .unwrap();
    journal
        .change_session_stop(&original.attempt, SessionStopChange::Unconfirmed)
        .unwrap();
    let mut end = ended();
    end.node = original.node.clone();
    journal
        .defer_session_stop_end(&input.execution_id, &end)
        .unwrap();
    assert!(
        journal
            .end_session(&input.execution_id, end.clone())
            .is_err()
    );
    drop(journal);
    drop(db);
    let db = NodeDatabase::open(&path, NodeIdentity::Discover).unwrap();
    let journal = db.session_journal().unwrap();
    journal
        .change_session_stop(
            &original.attempt,
            SessionStopChange::Request {
                at_ms: 999,
                bound_at_ms: 5000,
            },
        )
        .unwrap();
    journal
        .defer_session_stop_end(&input.execution_id, &ended())
        .unwrap();
    let record = journal
        .session_stop_records(Some(&input.execution_id))
        .unwrap()
        .remove(0);
    assert_eq!(record.identity, original);
    assert_eq!(
        (record.first_stop_at_ms, record.bound_at_ms),
        (Some(20), Some(620))
    );
    assert_eq!(
        journal.session_stop_end(&input.execution_id).unwrap(),
        Some(end.clone())
    );
    let status = journal
        .session_stop_unconfirmed(&input.execution_id)
        .unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].run_id, original.attempt.as_str());
    journal
        .change_session_stop(
            &original.attempt,
            SessionStopChange::Confirm {
                at_ms: 1000,
                proof: SessionStopProof::LockAndProcesses,
            },
        )
        .unwrap();
    assert!(
        journal
            .end_session(&input.execution_id, end.clone())
            .is_err()
    );
    journal
        .change_session_stop(&original.attempt, SessionStopChange::ResourcesReleased)
        .unwrap();
    journal.end_session(&input.execution_id, end).unwrap();
}

#[test]
fn audit_is_root_only_idempotent_and_commits_proof_before_release() {
    let directory = tempfile::tempdir().unwrap();
    let (_db, journal, input) = running(&directory.path().join("db"));
    let original = identity(&input);
    journal.record_session_stop(&original, 1).unwrap();
    let request = SessionStopReconcile {
        request_id: "recheck-1".into(),
        execution: input.execution_id,
        attempt: original.attempt.clone(),
        reason: SessionStopReason::Maintenance,
        mode: SessionStopMode::Recheck,
    };
    assert!(
        journal
            .begin_session_stop_audit(1000, &request, 10)
            .is_err()
    );
    assert_eq!(
        journal.begin_session_stop_audit(0, &request, 10).unwrap(),
        SessionStopAuditAdmission::Started
    );
    assert_eq!(
        journal.begin_session_stop_audit(0, &request, 11).unwrap(),
        SessionStopAuditAdmission::InProgress
    );
    assert_eq!(
        journal.incomplete_session_stop_audits().unwrap(),
        vec![request.clone()]
    );
    let result = SessionStopAuditResult {
        observation: SessionStopObservation::Stopped,
        before: SessionStopState::Prepared,
        after: SessionStopState::Confirmed,
        proof: Some(SessionStopProof::LockAndProcesses),
        released: vec![],
    };
    journal
        .finish_session_stop_audit(&request, &result, 20)
        .unwrap();
    assert_eq!(
        journal.begin_session_stop_audit(0, &request, 30).unwrap(),
        SessionStopAuditAdmission::Completed(result.clone())
    );
    let mut conflict = request.clone();
    conflict.reason = SessionStopReason::LegacyUpgrade;
    assert!(journal.begin_session_stop_audit(0, &conflict, 31).is_err());
    assert!(!journal.session_stop_records(None).unwrap()[0].resources_released);
    assert!(journal.incomplete_session_stop_audits().unwrap().is_empty());
}

#[test]
fn undrained_v13_upgrade_marks_only_running_agents_legacy_unknown() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (mut db, journal, input) = running(&path);
    let mut accepted = input.clone();
    accepted.execution_id = ExecutionId::new("accepted");
    accepted.operation_id = OperationId::new("accepted-op");
    db.accept_session(&accepted).unwrap();
    let mut completed = input.clone();
    completed.execution_id = ExecutionId::new("completed");
    completed.operation_id = OperationId::new("completed-op");
    db.accept_session(&completed).unwrap();
    journal
        .end_session(&completed.execution_id, ended())
        .unwrap();
    drop(journal);
    drop(db);
    let old = Connection::open(&path).unwrap();
    remove_session_stop_schema(&old);
    old.pragma_update(None, "user_version", 13).unwrap();
    drop(old);
    let host = SessionStopHost {
        boot_id: "upgrade-boot".into(),
        trusted_host_id: Some("physical-host".into()),
    };
    let db =
        NodeDatabase::open_with_guard_and_host(&path, NodeIdentity::Discover, DurableWrites, &host)
            .unwrap();
    let journal = db.session_journal().unwrap();
    let records = journal.session_stop_records(None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].identity.execution, input.execution_id);
    assert_eq!(records[0].identity.host, host);
    assert_eq!(records[0].state, SessionStopState::LegacyUnknown);
    journal
        .change_session_stop(
            &records[0].identity.attempt,
            SessionStopChange::Request {
                at_ms: 10,
                bound_at_ms: 20,
            },
        )
        .unwrap();
    journal
        .change_session_stop(&records[0].identity.attempt, SessionStopChange::Unconfirmed)
        .unwrap();
    assert_eq!(
        journal.session_stop_records(None).unwrap()[0].state,
        SessionStopState::LegacyUnknown,
        "ordinary stop processing cannot erase the stronger upgrade fence"
    );
    assert!(
        journal
            .change_session_stop(
                &records[0].identity.attempt,
                SessionStopChange::Confirm {
                    at_ms: 1,
                    proof: SessionStopProof::LockAndProcesses
                }
            )
            .is_err()
    );
    journal
        .change_session_stop(
            &records[0].identity.attempt,
            SessionStopChange::Confirm {
                at_ms: 2,
                proof: SessionStopProof::HostReboot,
            },
        )
        .unwrap();
}

#[test]
fn gc_requires_age_release_ack_and_completed_audit_and_retains_a_summary() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (mut db, journal, input) = running(&path);
    let original = identity(&input);
    journal.record_session_stop(&original, 1).unwrap();
    let later = 8 * 24 * 60 * 60 * 1000;
    assert!(
        journal
            .session_stop_gc_candidates(later)
            .unwrap()
            .is_empty()
    );
    journal
        .change_session_stop(
            &original.attempt,
            SessionStopChange::Confirm {
                at_ms: 10,
                proof: SessionStopProof::LockAndProcesses,
            },
        )
        .unwrap();
    journal
        .change_session_stop(&original.attempt, SessionStopChange::ResourcesReleased)
        .unwrap();
    journal.end_session(&input.execution_id, ended()).unwrap();
    assert!(
        journal
            .session_stop_gc_candidates(later)
            .unwrap()
            .is_empty()
    );
    db.acknowledge(&ack(1)).unwrap();
    assert!(journal.session_stop_gc_candidates(100).unwrap().is_empty());
    let request = SessionStopReconcile {
        request_id: "gc-audit".into(),
        execution: input.execution_id.clone(),
        attempt: original.attempt.clone(),
        reason: SessionStopReason::Maintenance,
        mode: SessionStopMode::Recheck,
    };
    journal.begin_session_stop_audit(0, &request, 20).unwrap();
    assert!(
        journal
            .retire_session_stop(&original.attempt, later)
            .is_err()
    );
    journal
        .finish_session_stop_audit(
            &request,
            &SessionStopAuditResult {
                observation: SessionStopObservation::Stopped,
                before: SessionStopState::Confirmed,
                after: SessionStopState::Confirmed,
                proof: Some(SessionStopProof::LockAndProcesses),
                released: vec![],
            },
            30,
        )
        .unwrap();
    assert_eq!(
        journal.session_stop_gc_candidates(later).unwrap(),
        vec![original.attempt.clone()]
    );
    journal
        .retire_session_stop(&original.attempt, later)
        .unwrap();
    let inspect = Connection::open(&path).unwrap();
    assert_eq!(
        inspect
            .query_row(
                "SELECT confirmed_attempts FROM session_stop_summary",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        inspect
            .query_row("SELECT count(*) FROM session_stop_audit", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
