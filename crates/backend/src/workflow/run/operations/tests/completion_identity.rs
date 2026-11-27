use super::*;
use crate::workflow::run::interactive::{claim_node_for_completion, revalidate_completion};
use ora_db::workflow_debug::WorkflowDebugStore;
use pretty_assertions::assert_eq;

/// Builds a real preview whose parked target will be retired without a stop acknowledgement.
fn edit_parked(f: &Fixture) -> CommitWorkflowDebugRequest {
    let preview = f
        .runs
        .debug_preview(
            serde_json::from_value(serde_json::json!({
                "runId": "run-1", "nodeRunId": f.node_run_id, "baseRevision": 0,
                "patch": [{"op":"replace", "path":"/agentConfig/prompt", "value":"revised"}]
            }))
            .unwrap(),
        )
        .unwrap();
    assert!(preview.impact.stop.is_empty());
    assert!(preview.impact.invalidate.contains(&f.node_run_id));
    CommitWorkflowDebugRequest {
        run_id: "run-1".into(),
        preview_id: preview.preview_id,
        request_id: "parked-edit".into(),
    }
}

/// A parked attempt is fenced at both admission and final write, including a claim which
/// passed before the debug operation began.
#[test]
fn stopping_and_unreleased_failure_reject_parked_completions() {
    run_test(async {
        let f = Fixture::awaiting();
        let request = edit_parked(&f);
        let store = WorkflowDebugStore::new(f.runs.pool.clone());
        let run = WorkflowRunId::new("run-1");
        let claim = || {
            claim_node_for_completion(
                &f.runs.pool,
                &f.runs.run_locks,
                &f.runs.completing_node_runs,
                &run,
                "agent",
                Some(&f.node_run_id),
                &CompletionApproval::FORCE,
            )
        };
        let claimed = claim().unwrap();
        // Release the in-memory claim so a second claim can only be rejected by persisted state.
        f.runs.release_completion_claim(&claimed).await;
        store.claim(&request).unwrap();
        let before = f.detail();
        assert!(
            before
                .nodes
                .iter()
                .any(|node| node.id == f.node_run_id && node.status == WorkflowNodeStatus::Pending)
        );
        for phase in ["stopping", "stop_failed"] {
            if phase == "stop_failed" {
                assert!(
                    store
                        .record_stop_failure("run-1", &request.request_id, "isolation pending", &[])
                        .unwrap()
                );
            }
            let rejected = PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {});
            assert_eq!(claim().unwrap_err().public_error(), &rejected);
            assert_eq!(
                prepare_completion(
                    &f.runs.pool,
                    &f.runs.sessions_root,
                    &f.runs.baselines_root,
                    &f.workspace,
                    &run,
                    ("agent", &claimed),
                    CompletionApproval::FORCE
                )
                .unwrap_err()
                .public_error(),
                &rejected
            );
            // The fence can also arrive after prepare's read and before its repair UPDATE.
            let repository = SqliteWorkflowRunEngineRepository::new(f.runs.pool.clone());
            let original = repository.find_node_run_by_id(&claimed).unwrap();
            assert!(
                !repository
                    .repair_node_payload(
                        &claimed,
                        &serde_json::json!({"completionPolicy": "agent"}),
                        /*now*/ 60
                    )
                    .unwrap()
            );
            assert_eq!(repository.find_node_run_by_id(&claimed).unwrap(), original);
            assert_eq!(
                revalidate_completion(&f.runs.pool, &run, ("agent", &claimed))
                    .unwrap_err()
                    .public_error(),
                &rejected
            );
            let request = serde_json::from_value(serde_json::json!({
                "runId": "run-1", "nodeId": "agent", "nodeRunId": f.node_run_id, "force": true
            }))
            .unwrap();
            assert_eq!(
                f.runs
                    .complete_node(request)
                    .await
                    .unwrap_err()
                    .public_error(),
                &rejected
            );
            let detail = f.detail();
            assert_eq!(detail.nodes, before.nodes);
            assert_eq!(detail.run.status, before.run.status);
        }
    });
}

/// Retiring the attempt through a real debug commit invalidates an already claimed completion.
#[test]
fn debug_commit_invalidates_a_claimed_completion() {
    run_test(async {
        let f = Fixture::awaiting();
        let edit = edit_parked(&f);
        let run = WorkflowRunId::new("run-1");
        let old = claim_node_for_completion(
            &f.runs.pool,
            &f.runs.run_locks,
            &f.runs.completing_node_runs,
            &run,
            "agent",
            Some(&f.node_run_id),
            &CompletionApproval::FORCE,
        )
        .unwrap();
        let receipt = f.runs.debug_commit(edit).await.unwrap();
        assert_eq!(receipt.phase, WorkflowDebugPhase::Applied { revision: 1 });
        let projection = f.detail();
        let replacement = projection
            .nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .unwrap();
        assert_ne!(replacement.id, old.to_string());
        let rejected = PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {});
        assert_eq!(
            prepare_completion(
                &f.runs.pool,
                &f.runs.sessions_root,
                &f.runs.baselines_root,
                &f.workspace,
                &run,
                ("agent", &old),
                CompletionApproval::FORCE
            )
            .unwrap_err()
            .public_error(),
            &rejected
        );
        assert_eq!(
            revalidate_completion(&f.runs.pool, &run, ("agent", &old))
                .unwrap_err()
                .public_error(),
            &rejected
        );
    });
}

/// A valid attempt id cannot substitute for either the requested graph node or its run.
#[test]
fn completion_rejects_mismatched_node_and_run() {
    run_test(async {
        let f = Fixture::awaiting();
        f.settle_turn();
        let run = WorkflowRunId::new("run-1");
        let old = WorkflowNodeRunId::new(&f.node_run_id);
        let rejected = PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {});
        assert_eq!(
            claim_node_for_completion(
                &f.runs.pool,
                &f.runs.run_locks,
                &f.runs.completing_node_runs,
                &run,
                "start",
                Some(old.as_ref()),
                &CompletionApproval::FORCE
            )
            .unwrap_err()
            .public_error(),
            &rejected
        );
        let repository = SqliteWorkflowRunRepository::new(f.runs.pool.clone());
        let mut other = repository.find_run(&run).unwrap().unwrap();
        other.id = WorkflowRunId::new("other-running-instance");
        let other_id = other.id.clone();
        repository.create_run(other).unwrap();
        assert_eq!(
            prepare_completion(
                &f.runs.pool,
                &f.runs.sessions_root,
                &f.runs.baselines_root,
                &f.workspace,
                &other_id,
                ("agent", &old),
                CompletionApproval::FORCE
            )
            .unwrap_err()
            .public_error(),
            &rejected
        );
    });
}

/// A root edit fences the first-round writer at every completion gate, including legacy claims.
#[test]
fn upstream_edit_rejects_parked_loop_member_writes_in_every_phase() {
    use crate::workflow::run::{debug::strategy, interactive::begin_human_turn, test_fixture};
    run_test(async {
        for phase in ["stopping", "stop_failed", "applied"] {
            let temp = TempDir::new().unwrap();
            let backend = Backend::open(backend_paths(temp.path(), temp.path())).unwrap();
            let runs = backend.workflow_runs();
            let (upstream, writer) = test_fixture::parked_writer_after_upstream(&temp, &runs.pool);
            let run = &writer.run_id;
            let attempt = &writer.id;
            let claim = |observed| {
                claim_node_for_completion(
                    &runs.pool,
                    &runs.run_locks,
                    &runs.completing_node_runs,
                    run,
                    "writer",
                    observed,
                    &CompletionApproval::PLAIN,
                )
            };
            // The legacy lookup is unambiguous before the edit. Drop the in-memory claim
            // so that subsequent admissions exercise the persisted fence.
            assert_eq!(claim(None).unwrap(), *attempt);
            runs.release_completion_claim(attempt).await;
            let store = WorkflowDebugStore::new(runs.pool.clone());
            let preview = runs
                .debug_preview(
                    serde_json::from_value(serde_json::json!({
                        "runId":run, "nodeRunId":upstream, "baseRevision":0,
                        "patch":[{"op":"replace","path":"/agentConfig/prompt","value":"revised"}]
                    }))
                    .unwrap(),
                )
                .unwrap();
            store
                .claim(&CommitWorkflowDebugRequest {
                    run_id: run.to_string(),
                    preview_id: preview.preview_id,
                    request_id: "edit".into(),
                })
                .unwrap();
            match phase {
                "stop_failed" => assert!(
                    store
                        .record_stop_failure(run.as_ref(), "edit", "pending", &[])
                        .unwrap()
                ),
                "applied" => assert_eq!(
                    store
                        .apply_after_stop(run.as_ref(), "edit", 61, |tx, p| strategy::settle(
                            tx,
                            run.as_ref(),
                            61,
                            p
                        ))
                        .unwrap()
                        .phase,
                    WorkflowDebugPhase::Applied { revision: 1 }
                ),
                _ => {}
            }
            let repo = SqliteWorkflowRunRepository::new(runs.pool.clone());
            let detail = || repo.get_run_detail(run).unwrap().unwrap();
            let before = detail();
            let old = || {
                store
                    .attempt(&GetWorkflowDebugAttemptRequest {
                        run_id: run.to_string(),
                        node_run_id: attempt.to_string(),
                    })
                    .unwrap()
            };
            let original = old();
            assert_eq!(original.superseded, phase == "applied");
            let rejected = PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {});
            for observed in [None, Some(attempt.as_ref())] {
                assert_eq!(
                    claim(observed).unwrap_err().public_error(),
                    &rejected,
                    "{phase}"
                );
            }
            assert_eq!(
                prepare_completion(
                    &runs.pool,
                    &runs.sessions_root,
                    &runs.baselines_root,
                    &temp.path().join("fixture-project"),
                    run,
                    ("writer", attempt),
                    CompletionApproval::FORCE
                )
                .unwrap_err()
                .public_error(),
                &rejected,
                "{phase}"
            );
            assert!(
                !SqliteWorkflowRunEngineRepository::new(runs.pool.clone())
                    .repair_node_payload(
                        attempt,
                        &serde_json::json!({"completionPolicy":"agent"}),
                        65
                    )
                    .unwrap(),
                "{phase}"
            );
            assert_eq!(
                revalidate_completion(&runs.pool, run, ("writer", attempt))
                    .unwrap_err()
                    .public_error(),
                &rejected,
                "{phase}"
            );
            let request = serde_json::from_value(serde_json::json!({
                "runId":run, "nodeId":"writer","nodeRunId":attempt,"force":true
            }))
            .unwrap();
            assert_eq!(
                runs.complete_node(request)
                    .await
                    .unwrap_err()
                    .public_error(),
                &rejected,
                "{phase}"
            );
            assert_eq!(
                begin_human_turn(
                    &runs.pool,
                    &runs.run_locks,
                    &runs.completing_node_runs,
                    &runs.transitions,
                    "session-1"
                )
                .await
                .unwrap_err()
                .public_error(),
                &rejected,
                "{phase}"
            );
            assert_eq!(detail(), before, "{phase}");
            assert_eq!(old(), original, "{phase}");
        }
    });
}
