use super::*;
use ora_db::DatabaseLocation;
use pretty_assertions::assert_eq;
use tokio::sync::Notify;

/// Holds the runtime's actual stop future across a concurrent cancellation.
#[derive(Default)]
struct StopBarrier {
    entered: Notify,
    release: Notify,
}
impl DebugSessionStopper for StopBarrier {
    fn stop(
        &self,
        _: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), BackendError>> + Send + '_>> {
        Box::pin(async {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(())
        })
    }
}

/// Simulates the already persisted terminal run, including values the abandon path must retain.
fn terminal(f: &Fixture, status: WorkflowRunStatus) {
    rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap().execute("UPDATE workflow_runs SET run_status=?1,error='original',output='original output',finished_at=321 WHERE id=?2",rusqlite::params![status.database_value(),f.run_id.as_ref()]).unwrap();
}

#[test]
fn cancellation_wins_while_stop_acknowledgement_is_in_flight() {
    run_test(async {
        for status in [
            WorkflowRunStatus::Running,
            WorkflowRunStatus::Failed,
            WorkflowRunStatus::Succeeded,
            WorkflowRunStatus::Cancelled,
        ] {
            let barrier = Arc::new(StopBarrier::default());
            let f = Fixture::new(barrier.clone());
            if status != WorkflowRunStatus::Running {
                terminal(&f, status);
            }
            let old = f.row("x");
            let req = f.commit_request(&f.preview(old.id.as_ref(), 0), "race");
            let result = tokio::join!(f.coordinator.commit(req.clone()), async {
                barrier.entered.notified().await;
                let result = f.engine.cancel(&f.run_id).unwrap();
                assert_eq!(
                    result,
                    if status == WorkflowRunStatus::Running {
                        ora_application::CancelWorkflowRunResult::Cancelled
                    } else {
                        ora_application::CancelWorkflowRunResult::DebugAbandoned
                    }
                );
                let receipt = f
                    .store
                    .history(f.run_id.as_ref())
                    .unwrap()
                    .remove(0)
                    .operation;
                assert_eq!(
                    receipt.phase,
                    WorkflowDebugPhase::StopFailed {
                        reason: "run_cancelled".into(),
                        unsettled_node_run_ids: vec![old.id.to_string()]
                    }
                );
                assert!(f.coordinator.ensure_no_fence(f.run_id.as_ref()).is_err());
                barrier.release.notify_one();
            })
            .0
            .unwrap();
            assert_eq!(f.coordinator.commit(req).await.unwrap(), result);
            assert!(f.coordinator.ensure_no_fence(f.run_id.as_ref()).is_ok());
            f.engine
                .complete_node(&f.run_id, &old.id, Some("late".into()), None, None, vec![])
                .unwrap();
            assert_eq!(dispatched(&f.executor, "x"), 1);
            assert!(out_rows(&f.pool, &f.run_id).is_empty());
            if status == WorkflowRunStatus::Running {
                assert_eq!(f.run_status(), WorkflowRunStatus::Cancelled);
            } else {
                let r = SqliteWorkflowRunRepository::new(f.pool.clone())
                    .find_run(&f.run_id)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    (
                        r.status,
                        r.error.as_deref(),
                        r.output.as_deref(),
                        r.finished_at
                    ),
                    (status, Some("original"), Some("original output"), Some(321))
                );
            }
        }
    });
}

/// Runs the production sweep in a new OS process using only the database path. No engine,
/// executor registry, timer or lock is shared with the process which accepted the operation.
fn boot_process(temp: &tempfile::TempDir, expected_calls: usize) {
    boot_with_calls(temp, if expected_calls == 0 { "" } else { "x" });
}

/// Pass the ordered dispatch expectation to the isolated restart worker.
fn boot_with_calls(temp: &tempfile::TempDir, expected: &str) {
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "workflow::run::debug_tests::lifecycle::restart_worker",
            "--nocapture",
        ])
        .env(
            "ORA_DEBUG_RESTART_FIXTURE",
            temp.path().join("repository.sqlite3"),
        )
        .env("ORA_DEBUG_RESTART_CALLS", expected)
        .output()
        .unwrap();
    assert!(
        result.status.success() && String::from_utf8_lossy(&result.stdout).contains("1 passed"),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn restart_worker() {
    let Some(path) = std::env::var_os("ORA_DEBUG_RESTART_FIXTURE") else {
        return;
    };
    run_test(async {
        let pool =
            RepositoryPool::new(&DatabaseLocation::path(std::path::PathBuf::from(path))).unwrap();
        let executor = RecordingExecutor::default();
        let engine = Arc::new(WorkflowRunEngine::new(
            SqliteWorkflowRunEngineRepository::new(pool.clone()),
            executor.clone(),
            UuidWorkflowNodeRunIdGenerator::new(),
            SystemClock,
        ));
        super::super::recovery::run_workflow_run_boot_sweep(
            &pool,
            &engine,
            &KeyedResourceLocks::new(),
            SystemClock,
        );
        assert_eq!(
            executor
                .records()
                .iter()
                .map(|r| r.node_id.as_str())
                .collect::<Vec<_>>()
                .join(","),
            std::env::var("ORA_DEBUG_RESTART_CALLS").unwrap()
        );
    });
}

#[test]
fn restart_quarantines_unproven_old_attempt_and_preserves_first_failure() {
    run_test(async {
        for status in [
            WorkflowRunStatus::Running,
            WorkflowRunStatus::Failed,
            WorkflowRunStatus::Succeeded,
            WorkflowRunStatus::Cancelled,
        ] {
            let f = Fixture::new(Arc::new(NoSessions));
            let s = f.row("s");
            f.engine
                .complete_node(
                    &f.run_id,
                    &s.id,
                    Some("independent".into()),
                    None,
                    None,
                    vec![],
                )
                .unwrap();
            if status != WorkflowRunStatus::Running {
                terminal(&f, status);
            }
            let old = f.row("x");
            let req = f.commit_request(&f.preview(old.id.as_ref(), 0), "restart");
            f.store.claim(&req).unwrap();
            boot_process(&f._temp, 0);
            let failed = f.coordinator.commit(req.clone()).await.unwrap();
            assert!(
                matches!(&failed.phase,WorkflowDebugPhase::StopFailed {reason,unsettled_node_run_ids} if reason.contains("isolation") && unsettled_node_run_ids==&[old.id.to_string()])
            );
            assert_eq!(
                (f.run_status(), f.row("s").status),
                (status, WorkflowNodeStatus::Succeeded)
            );
            f.engine
                .complete_node(&f.run_id, &old.id, Some("late".into()), None, None, vec![])
                .unwrap();
            assert!(out_rows(&f.pool, &f.run_id).is_empty());
            f.engine.cancel(&f.run_id).unwrap();
            boot_process(&f._temp, 0);
            assert_eq!(f.coordinator.commit(req).await.unwrap(), failed);
        }
    });
}

#[test]
fn restart_dispatches_unclaimed_once_but_never_reclaims_an_interrupted_attempt() {
    run_test(async {
        for cancel in [false, true] {
            let f = Fixture::new(Arc::new(NoSessions));
            let s = f.row("s");
            f.engine
                .complete_node(&f.run_id, &s.id, Some("ok".into()), None, None, vec![])
                .unwrap();
            let req = f.commit_request(&f.preview(f.row("x").id.as_ref(), 0), "apply");
            f.store.claim(&req).unwrap();
            let applied = f
                .store
                .apply_after_stop(
                    f.run_id.as_ref(),
                    "apply",
                    60,
                    super::super::debug::strategy::settle,
                )
                .unwrap();
            if cancel {
                f.engine.cancel(&f.run_id).unwrap();
            }
            boot_process(&f._temp, usize::from(!cancel));
            if cancel {
                assert_eq!(f.run_status(), WorkflowRunStatus::Cancelled);
            } else {
                let claimed = f.row("x");
                assert_eq!(claimed.status, WorkflowNodeStatus::Running);
                boot_process(&f._temp, 0);
                let interrupted = f.row("x");
                assert_eq!(interrupted.id, claimed.id);
                assert_eq!(interrupted.status, WorkflowNodeStatus::Failed);
                assert!(
                    interrupted
                        .error
                        .unwrap()
                        .contains("interrupted_by_restart")
                );
            }
            assert_eq!(f.coordinator.commit(req).await.unwrap(), applied);
        }
    });
}

/// A confirmed container plan is supplied by its strategy. Boot cleanup must observe that plan
/// ahead of the normal while interruption / foreach failure-ledger policy.
#[test]
fn restart_preserves_the_old_container_until_isolation_is_proven() {
    use super::super::{iteration_tests, loop_resume_tests};
    run_test(async {
        for (member, owner) in [("writer", "loop"), ("fix", "iter")] {
            let (temp, pool) = bootstrap();
            let id = if owner == "loop" {
                seeded_pending_run(&temp, &pool, loop_resume_tests::LOOP_GRAPH)
            } else {
                iteration_tests::seeded_iteration_run(
                    &temp,
                    &pool,
                    &iteration_tests::iteration_graph("agent", "continue", 3),
                    json!([{"id":1},{"id":2}]),
                )
            };
            let repo = SqliteWorkflowRunEngineRepository::new(pool.clone());
            let engine = WorkflowRunEngine::new(
                repo.clone(),
                RecordingExecutor::default(),
                UuidWorkflowNodeRunIdGenerator::new(),
                SystemClock,
            );
            engine.start(&id).unwrap();
            let before = node_runs(&pool, &id);
            let selected = before.iter().find(|r| r.node_id == member).unwrap();
            let parent = before.iter().find(|r| r.node_id == owner).unwrap();
            let store = WorkflowDebugStore::new(pool.clone());
            store
                .record_agent_dispatch(
                    id.as_ref(),
                    selected.id.as_ref(),
                    &json!({"variablePool": WorkflowVariablePool::default()}),
                )
                .unwrap();
            let request=serde_json::from_value(json!({"runId":id.as_ref(),"nodeRunId":selected.id.as_ref(),"baseRevision":0,"patch":[{"op":"replace","path":"/agentConfig/prompt","value":"revised"}]})).unwrap();
            let p = store
                .preview(&request, |_| {
                    Ok(ora_contracts::WorkflowDebugImpact {
                        keep: vec![],
                        invalidate: vec![selected.id.to_string()],
                        stop: vec![selected.id.to_string()],
                        schedule: vec![member.into()],
                        predicted_branch: None,
                    })
                })
                .unwrap();
            let req = CommitWorkflowDebugRequest {
                run_id: id.to_string(),
                preview_id: p.preview_id,
                request_id: "container".into(),
            };
            store.claim(&req).unwrap();
            let context = repo.find_execution_context(&id).unwrap().unwrap();
            let round = repo.find_active_loop_round(&parent.id).unwrap();
            boot_process(&temp, 0);
            engine.resume(&id).unwrap();
            assert_eq!(repo.find_active_loop_round(&parent.id).unwrap(), round);
            assert_eq!(repo.find_execution_context(&id).unwrap().unwrap(), context);
            assert_eq!(node_runs(&pool, &id), before);
            assert!(matches!(
                store.history(id.as_ref()).unwrap()[0].operation.phase,
                WorkflowDebugPhase::StopFailed { .. }
            ));
        }
    });
}

/// Reopen a failed run with no external stop outstanding. A ready sibling with no row was never
/// authorized: boot must dispatch only X, and its success alone cannot clear the original error.
#[test]
fn restart_finishes_a_provably_quiet_intent_without_expanding_its_scope() {
    run_test(async {
        let f = Fixture::new(Arc::new(NoSessions));
        let x = f.row("x");
        f.engine
            .complete_node(&f.run_id, &x.id, Some("old".into()), None, None, vec![])
            .unwrap();
        let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
        db.execute(
            "DELETE FROM workflow_node_runs WHERE id=?1",
            [f.row("s").id.as_ref()],
        )
        .unwrap();
        terminal(&f, WorkflowRunStatus::Failed);
        let preview = f.preview(x.id.as_ref(), 0);
        assert!(preview.impact.stop.is_empty());
        f.store.claim(&f.commit_request(&preview, "quiet")).unwrap();
        boot_process(&f._temp, 1);
        assert_eq!(f.run_status(), WorkflowRunStatus::Running);
        let new_x = f.row("x");
        assert_ne!(x.id, new_x.id);
        f.engine
            .complete_node(
                &f.run_id,
                &new_x.id,
                Some("fixed".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        let run = SqliteWorkflowRunRepository::new(f.pool.clone())
            .find_run(&f.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            (run.status, run.error.as_deref()),
            (WorkflowRunStatus::Failed, Some("original"))
        );
        assert!(
            node_runs(&f.pool, &f.run_id)
                .iter()
                .all(|r| r.node_id != "s")
        );
        assert_eq!(
            f.store.history(f.run_id.as_ref()).unwrap()[0]
                .operation
                .phase,
            WorkflowDebugPhase::Applied { revision: 1 }
        );
    });
}

/// Overlapping edits keep the first terminal outcome and authorize both previews' positions.
#[test]
fn overlapping_confirmations_preserve_terminal_outcome_and_schedule() {
    run_test(async {
        for second_target in ["x", "s"] {
            let f = Fixture::new(Arc::new(NoSessions));
            let complete = |node| {
                f.engine
                    .complete_node(
                        &f.run_id,
                        &f.row(node).id,
                        Some("done".into()),
                        None,
                        None,
                        vec![],
                    )
                    .unwrap();
            };
            complete("x");
            f.engine
                .fail_node(
                    &f.run_id,
                    &f.row("s").id,
                    ora_application::NodeFailure::new(
                        ora_application::NodeFailureKind::Session,
                        "independent failure",
                    ),
                )
                .unwrap();
            let repository = SqliteWorkflowRunRepository::new(f.pool.clone());
            let original = repository.find_run(&f.run_id).unwrap().unwrap();
            assert_eq!(original.status, WorkflowRunStatus::Failed);
            let first = f.preview(f.row("x").id.as_ref(), 0);
            f.coordinator
                .commit(f.commit_request(&first, "first"))
                .await
                .unwrap();
            let active_x = f.row("x");
            SqliteWorkflowRunEngineRepository::new(f.pool.clone())
                .bind_node_run_session(&active_x.id, &SessionId::new("session-1"), 60)
                .unwrap();
            f.store
                .record_agent_dispatch(
                    f.run_id.as_ref(),
                    active_x.id.as_ref(),
                    &json!({"variablePool":WorkflowVariablePool::default()}),
                )
                .unwrap();
            let second = f.preview(f.row(second_target).id.as_ref(), 1);
            let receipt = f
                .coordinator
                .commit(f.commit_request(&second, "second"))
                .await
                .unwrap();
            assert_eq!(receipt.phase, WorkflowDebugPhase::Applied { revision: 2 });
            complete("x");
            if second_target == "x" {
                let actual = repository.find_run(&f.run_id).unwrap().unwrap();
                assert_eq!(
                    (
                        actual.status,
                        actual.error,
                        actual.output,
                        actual.finished_at
                    ),
                    (
                        original.status,
                        original.error,
                        original.output,
                        original.finished_at
                    )
                );
                assert_eq!(f.row("s").status, WorkflowNodeStatus::Failed);
            } else {
                assert_eq!(f.row("out").status, WorkflowNodeStatus::Succeeded);
                complete("s");
                assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
            }
        }
    });
}

/// An actual backoff wake arriving on either side of restart must not claim the retired row.
#[test]
fn delayed_retry_cannot_reclaim_the_old_attempt_after_boot() {
    use super::super::retry_tests::Harness;
    run_test(async {
        let graph = GRAPH
            .replace(
                "\"enabled\":false,\"maxRetries\":0",
                "\"enabled\":true,\"maxRetries\":2",
            )
            .replace(
                r#""source":"start","target":"x""#,
                r#""source":"s","target":"x""#,
            );
        let h = Harness::start(&graph);
        let store = WorkflowDebugStore::new(h.pool.clone());
        let s = h.running("s");
        store
            .record_agent_dispatch(
                h.run_id.as_ref(),
                s.id.as_ref(),
                &json!({"variablePool":WorkflowVariablePool::default()}),
            )
            .unwrap();
        h.complete(&s.id, "ok", 50);
        let first = h.running("x");
        h.fail(&first.id, ora_application::NodeFailureKind::Session, 60);
        let waiting = h.running("x");
        assert_ne!(waiting.id, first.id);
        assert!(Harness::wait_of(&waiting).is_some());
        // Edit S, the successful predecessor of X. The normal plan must include its waiting retry.
        let req=serde_json::from_value(json!({"runId":h.run_id.as_ref(),"nodeRunId":s.id.as_ref(),"baseRevision":0,"patch":[{"op":"replace","path":"/agentConfig/prompt","value":"next"}]})).unwrap();
        let plan = store
            .preview(&req, super::super::debug::strategy::plan)
            .unwrap();
        assert_eq!(plan.impact.stop, vec![waiting.id.to_string()]);
        store
            .claim(&CommitWorkflowDebugRequest {
                run_id: h.run_id.to_string(),
                preview_id: plan.preview_id,
                request_id: "k".into(),
            })
            .unwrap();
        h.wake(&waiting.id, 10000);
        assert_eq!(h.dispatched("x"), vec![first.id.to_string()]);
        boot_with_calls(&h.temp, "s");
        h.wake(&waiting.id, 15000);
        assert!(h.rows_of("x").is_empty());
        h.complete(&h.running("s").id, "new", 16000);
        let replacement = h.running("x");
        assert_ne!(replacement.id, waiting.id);
        h.wake(&waiting.id, 20000);
        assert_eq!(h.running("x").id, replacement.id);
        assert_eq!(
            h.dispatched("x"),
            vec![first.id.to_string(), replacement.id.to_string()]
        );
        assert_eq!(
            store.history(h.run_id.as_ref()).unwrap()[0].operation.phase,
            WorkflowDebugPhase::Applied { revision: 1 }
        );
    });
}
