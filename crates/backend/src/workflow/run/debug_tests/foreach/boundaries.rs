use super::*;
use ora_application::{NodeFailure, NodeFailureKind, NodeRetryToSchedule, WorkflowRetryRepository};
use pretty_assertions::assert_eq;

/// Two sequential foreach containers let the public scheduler prove both member and downstream replay.
fn batch(stopper: Arc<dyn DebugSessionStopper>) -> Fixture {
    batch_graph(stopper, false)
}

fn batch_graph(stopper: Arc<dyn DebugSessionStopper>, sibling: bool) -> Fixture {
    let graph = batch_definition(sibling);
    Fixture::with_inputs(
        stopper,
        &graph.to_string(),
        [
            ("items".into(), json!(["a", "b", "c", "d", "e"])),
            ("objects".into(), json!([{"label":"new"}])),
        ]
        .into(),
    )
}

/// Build the iteration graph so recovery tests can add an independent container.
fn batch_definition(sibling: bool) -> serde_json::Value {
    let agent = |name: &str, parent: &str| {
        json!({"id":name,"parentId":parent,"data":{"kind":"agent","agentConfig":{
        "executor":{"agentCli":"c","modelId":"m"},"prompt":"old","outputContract":{"type":"structured","textExposure":"includeFinalText","schema":{"type":"object"}},"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0}}}})
    };
    let mut graph = json!({"nodes":[{"id":"start","data":{"kind":"start","inputVariables":[
        {"name":"items","valueType":"array[string]"},{"name":"objects","valueType":"array[object]"}]}},
        {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","items"],"collectSelector":["member","output"],"errorStrategy":"fail"}}},
        agent("member","iter"),
        {"id":"next","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["iter","output"],"collectSelector":["consumer","output"],"errorStrategy":"fail"}}},
        agent("consumer","next")],
        "edges":[{"source":"start","target":"iter"},{"source":"iter","target":"member"},
            {"source":"iter","target":"next"},{"source":"next","target":"consumer"}]});
    if sibling {
        graph["nodes"][1]["data"]["iterationConfig"]["errorStrategy"] = json!("continue");
        let mut independent = agent("sibling", "");
        independent.as_object_mut().unwrap().remove("parentId");
        graph["nodes"].as_array_mut().unwrap().push(independent);
        graph["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":"start","target":"sibling"}));
    }
    graph
}

/// Reads the effective projection through the same repository as the scheduler.
fn payload(f: &Fixture) -> WorkflowRunPayload {
    let run = SqliteWorkflowRunRepository::new(f.pool.clone())
        .find_run(&f.run_id)
        .unwrap()
        .unwrap();
    serde_json::from_str(run.payload.as_deref().unwrap()).unwrap()
}

/// Selects a live member in one item, rather than the earliest row with that definition.
fn active(f: &Fixture, name: &str) -> WorkflowNodeRun {
    node_runs(&f.pool, &f.run_id)
        .into_iter()
        .find(|r| r.node_id == name && r.status == WorkflowNodeStatus::Running)
        .unwrap()
}

/// Releases the recording executor's pending member so the next ready position dispatches.
fn complete(f: &Fixture, name: &str, value: &str) {
    f.engine
        .complete_node(
            &f.run_id,
            &active(f, name).id,
            Some(value.into()),
            None,
            None,
            vec![],
        )
        .unwrap();
}

/// Saves the dispatch input a real Agent session would have recorded.
fn evidence(f: &Fixture, row: &WorkflowNodeRun, pool: &WorkflowVariablePool) {
    assert!(
        f.store
            .record_agent_dispatch(
                f.run_id.as_ref(),
                row.id.as_ref(),
                &json!({"variablePool":pool})
            )
            .unwrap()
    );
}

/// Binds the in-flight Agent to the controllable stopper.
fn bind(f: &Fixture, row: &WorkflowNodeRun) {
    SqliteWorkflowRunEngineRepository::new(f.pool.clone())
        .bind_node_run_session(&row.id, &SessionId::new("session-1"), 50)
        .unwrap();
}

/// D3 edits the third in-flight call; its late completion cannot publish output or advance the batch.
#[test]
fn third_in_flight_rejects_late_result_and_dispatches_only_the_selected_item_twice() {
    run_test(async {
        let f = batch(Arc::new(NoSessions));
        complete(&f, "member", "one");
        complete(&f, "member", "two");
        let old = active(&f, "member");
        evidence(&f, &old, &payload(&f).variable_pool);
        bind(&f, &old);
        let preview = f.preview(old.id.as_ref(), 0);
        f.coordinator
            .commit(f.commit_request(&preview, "third"))
            .await
            .unwrap();
        let replacement = active(&f, "member");
        assert_ne!(replacement.id, old.id);
        let _late =
            f.engine
                .complete_node(&f.run_id, &old.id, Some("late".into()), None, None, vec![]);
        assert_eq!(active(&f, "member").id, replacement.id);
        for val in ["three", "four", "five"] {
            complete(&f, "member", val);
        }
        assert_eq!(
            payload(&f).variable_pool.values["iter.output"],
            json!(["one", "two", "three", "four", "five"])
        );
        let mut counts = [0; 5];
        for call in f
            .executor
            .records()
            .into_iter()
            .filter(|r| r.node_id == "member")
        {
            let state: WorkflowRunPayload =
                serde_json::from_str(call.payload.as_deref().unwrap()).unwrap();
            counts[state.variable_pool.values["iter.index"].as_u64().unwrap() as usize] += 1;
        }
        assert_eq!(counts, [1, 1, 2, 1, 1]);
    });
}

/// Preview must prove both the item coordinates and every settled predecessor index.
#[test]
fn preview_rejects_wrong_index_missing_item_and_a_gap_in_the_prefix() {
    run_test(async {
        for defect in ["index", "item", "prefix"] {
            let f = batch(Arc::new(NoSessions));
            complete(&f, "member", "one");
            let target = active(&f, "member");
            let mut state = payload(&f);
            let mut input = state.variable_pool.clone();
            match defect {
                "index" => {
                    input.values.insert("iter.index".into(), json!(4));
                }
                "item" => {
                    input.values.remove("iter.item");
                }
                _ => {
                    state.iteration_ledger.remove("iter");
                    rusqlite::Connection::open(f._temp.path().join("repository.sqlite3"))
                        .unwrap()
                        .execute(
                            "UPDATE workflow_runs SET payload=?2 WHERE id=?1",
                            (f.run_id.as_ref(), serde_json::to_string(&state).unwrap()),
                        )
                        .unwrap();
                }
            }
            evidence(&f, &target, &input);
            let err=f.coordinator.preview(serde_json::from_value(json!({"runId":f.run_id.as_ref(),"nodeRunId":target.id.as_ref(),"baseRevision":0,
                "patch":[{"op":"replace","path":"/agentConfig/prompt","value":"edited"}]})).unwrap()).unwrap_err();
            assert!(
                error_chain(&err).contains("missing_input_evidence"),
                "{defect}: {err:?}"
            );
            assert!(f.store.history(f.run_id.as_ref()).unwrap().is_empty());
        }
    });
}

/// The accepted retry source remains readable after the coordinator cancels the waiting row.
#[test]
fn confirm_a_waiting_member_uses_the_dispatch_bound_in_the_receipt() {
    run_test(async {
        let f = batch(Arc::new(NoSessions));
        let old = active(&f, "member");
        evidence(&f, &old, &payload(&f).variable_pool);
        let repo = SqliteWorkflowRunEngineRepository::new(f.pool.clone());
        let waiting = ora_domain::WorkflowNodeRunId::new("waiting");
        repo.schedule_node_retry(
            &old.id,
            &NodeFailure::new(NodeFailureKind::Session, "retry"),
            &NodeRetryToSchedule {
                node_run_id: waiting.clone(),
                retry: 1,
                max_retries: 1,
                delay_ms: 20,
                due_at: 30,
            },
            10,
        )
        .unwrap();
        let p = f.preview(waiting.as_ref(), 0);
        assert_eq!(p.input_source_node_run_id.as_deref(), Some(old.id.as_ref()));
        let op = f
            .coordinator
            .commit(f.commit_request(&p, "waiting"))
            .await
            .unwrap();
        assert_eq!(op.phase, WorkflowDebugPhase::Applied { revision: 1 });
        assert_ne!(active(&f, "member").id, waiting);
        assert_eq!(dispatched(&f.executor, "member"), 2);
    });
}

/// Deliberately lose dispatch evidence after the claim and before apply; replay must see a durable failure.
#[test]
fn unprovable_projection_rolls_back_the_revision_and_records_stop_failed() {
    run_test(async {
        let barrier = Arc::new(super::super::lifecycle::StopBarrier::default());
        let f = batch_graph(barrier.clone(), true);
        let repo = SqliteWorkflowRunEngineRepository::new(f.pool.clone());
        let wait = ora_domain::WorkflowNodeRunId::new("independent-wait");
        assert_eq!(
            repo.schedule_node_retry(
                &active(&f, "sibling").id,
                &NodeFailure::new(NodeFailureKind::Session, "retry later"),
                &NodeRetryToSchedule {
                    node_run_id: wait.clone(),
                    retry: 1,
                    max_retries: 1,
                    delay_ms: 60000,
                    due_at: 60010
                },
                10
            )
            .unwrap(),
            ora_application::ScheduleNodeRetryResult::Scheduled
        );
        complete(&f, "member", "one");
        let row = active(&f, "member");
        bind(&f, &row);
        evidence(&f, &row, &payload(&f).variable_pool);
        let p = f.preview(row.id.as_ref(), 0);
        let (op, notices_before_release) = tokio::join!(
            f.coordinator.commit(f.commit_request(&p, "broken")),
            async {
                barrier.entered.notified().await;
                rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap().execute(
                "DELETE FROM workflow_debug_documents WHERE node_run_id=?1 AND kind='dispatch'",[row.id.as_ref()]).unwrap();
                let notices = f.invalidations.0.lock().unwrap().len();
                barrier.release.notify_one();
                notices
            }
        );
        assert!(
            f.invalidations.0.lock().unwrap()[notices_before_release..]
                .contains(&f.run_id.to_string())
        );
        let op = op.unwrap();
        assert!(
            matches!(&op.phase,WorkflowDebugPhase::StopFailed{reason,..} if reason.contains("missing_input_evidence"))
        );
        assert_eq!(
            f.coordinator
                .commit(f.commit_request(&p, "broken"))
                .await
                .unwrap(),
            op
        );
        assert!(f.store.dispatch_fence(f.run_id.as_ref()).unwrap().is_none());
        let failed = SqliteWorkflowRunRepository::new(f.pool.clone())
            .find_run(&f.run_id)
            .unwrap()
            .unwrap();
        let WorkflowDebugPhase::StopFailed {
            reason,
            unsettled_node_run_ids,
        } = &op.phase
        else {
            unreachable!()
        };
        assert!(unsettled_node_run_ids.is_empty());
        assert_eq!(failed.status, WorkflowRunStatus::Failed);
        assert_eq!(failed.error.as_ref(), Some(reason));
        let abandoned = f.row("sibling");
        assert_eq!(abandoned.id, wait);
        assert_eq!(abandoned.status, WorkflowNodeStatus::Cancelled);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(abandoned.error.as_deref().unwrap()).unwrap()
                ["reason"],
            json!("retry_abandoned")
        );
        assert_eq!(dispatched(&f.executor, "sibling"), 1);
        assert!(failed.finished_at.is_some());
        assert_eq!(dispatched(&f.executor, "member"), 2);
        assert!(node_runs(&f.pool, &f.run_id).iter().any(|r| r.id == row.id));
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(active(&f, "member").iteration, Some(0));
    });
}

/// A failed run can finish successfully after replay; a subsequent entrance rebuild changes
/// the item type, clears both ledgers, and never reuses downstream entries from the old batch.
#[test]
fn failed_batch_recovers_then_rebuilds_with_an_object_source_and_empty_downstream_ledger() {
    run_test(async {
        let f = batch(Arc::new(NoSessions));
        complete(&f, "member", "first");
        let row = active(&f, "member");
        evidence(&f, &row, &payload(&f).variable_pool);
        f.engine
            .fail_node(
                &f.run_id,
                &row.id,
                NodeFailure::new(NodeFailureKind::Session, "original"),
            )
            .unwrap();
        assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
        let p = f.preview(row.id.as_ref(), 0);
        f.coordinator
            .commit(f.commit_request(&p, "failed"))
            .await
            .unwrap();
        for val in ["second", "third", "fourth", "fifth"] {
            complete(&f, "member", val);
        }
        for _ in 0..5 {
            complete(&f, "consumer", "old");
        }
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(payload(&f).iteration_ledger["next"].len(), 5);
        let p=f.coordinator.preview(serde_json::from_value(json!({"runId":f.run_id.as_ref(),"nodeRunId":f.row("iter").id.as_ref(),"baseRevision":1,
            "patch":[{"op":"replace","path":"/iterationConfig/iteratorSelector","value":["start","objects"]}]})).unwrap()).unwrap();
        f.coordinator
            .commit(f.commit_request(&p, "objects"))
            .await
            .unwrap();
        assert!(!payload(&f).iteration_ledger.contains_key("next"));
        assert_eq!(
            payload(&f).variable_pool.values["iter.item"],
            json!({"label":"new"})
        );
        complete(&f, "member", "updated");
        complete(&f, "consumer", "fresh");
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(
            payload(&f).variable_pool.values["next.output"],
            json!(["fresh"])
        );
        let p=f.coordinator.preview(serde_json::from_value(json!({"runId":f.run_id.as_ref(),"nodeRunId":f.row("iter").id.as_ref(),"baseRevision":2,
            "patch":[{"op":"replace","path":"/iterationConfig/collectSelector","value":["member","structured_output"]}]})).unwrap()).unwrap();
        f.coordinator
            .commit(f.commit_request(&p, "collect-object"))
            .await
            .unwrap();
        f.engine
            .complete_node(
                &f.run_id,
                &active(&f, "member").id,
                Some("text".into()),
                Some(json!({"value":7})),
                None,
                vec![],
            )
            .unwrap();
        assert_eq!(
            payload(&f).variable_pool.values["iter.output"],
            json!([{"value":7}])
        );
        assert_eq!(
            payload(&f).variable_pool.values["next.item"],
            json!({"value":7})
        );
        complete(&f, "consumer", "consumed object");
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
    });
}

mod recovery;

mod cross_container;
