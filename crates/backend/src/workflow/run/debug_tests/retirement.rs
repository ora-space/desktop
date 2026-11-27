use super::*;
use ora_application::{
    NodeFailure, NodeFailureKind, NodeRetryToSchedule, ScheduleNodeRetryResult,
    WorkflowRetryRepository,
};
use ora_domain::WorkflowNodeRunId;
use pretty_assertions::assert_eq;

/// Even a caller that passed the earlier engine-level fence check cannot replace the row
/// after claim. A retry on an independent branch still uses the normal transaction.
#[test]
fn retry_write_rechecks_the_durable_stop_set() {
    let f = Fixture::new(Arc::new(NoSessions));
    let x = f.row("x").id;
    let s = f.row("s").id;
    let preview = f.preview(x.as_ref(), 0);
    f.store.claim(&f.commit_request(&preview, "pause")).unwrap();
    let repo = SqliteWorkflowRunEngineRepository::new(f.pool.clone());
    let retry = |id, replacement| {
        repo.schedule_node_retry(
            id,
            &NodeFailure::new(NodeFailureKind::AgentRefusal, "retry"),
            &NodeRetryToSchedule {
                node_run_id: WorkflowNodeRunId::new(replacement),
                retry: 1,
                max_retries: 1,
                delay_ms: 100,
                due_at: 200,
            },
            100,
        )
        .unwrap()
    };
    assert_eq!(retry(&x, "retry-x"), ScheduleNodeRetryResult::NotRunning);
    assert_eq!(retry(&s, "retry-s"), ScheduleNodeRetryResult::Scheduled);
    assert_eq!(f.row("x").id, x);
    assert_eq!(f.row("s").id.as_ref(), "retry-s");
}

/// Restarting a container from upstream must stop its actual member and clear its
/// completed prefix, not just hide the top-level container row.
#[test]
fn editing_upstream_retires_active_item_and_completed_ledger() {
    run_test(async {
        let graph = r#"{"nodes":[
            {"id":"start","data":{"kind":"start","inputVariables":[{"name":"prs","valueType":"array[object]"}]}},
            {"id":"x","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"old"}}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","prs"],"collectSelector":["fix","output"],"errorStrategy":"continue","maxIterations":5}}},
            {"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"fix"}}}
        ],"edges":[{"source":"start","target":"x"},{"source":"x","target":"iter"},{"source":"iter","target":"fix"}]}"#;
        let f = Fixture::with_inputs(
            Arc::new(NoSessions),
            graph,
            [("prs".into(), json!([{"id":1},{"id":2}]))].into(),
        );
        let x = f.row("x").id;
        let complete = |id: &WorkflowNodeRunId| {
            f.engine
                .complete_node(&f.run_id, id, Some("done".into()), None, None, vec![])
                .unwrap()
        };
        complete(&x);
        complete(&f.row("fix").id);
        let member = node_runs(&f.pool, &f.run_id)
            .into_iter()
            .find(|r| r.node_id == "fix" && r.iteration == Some(1))
            .unwrap();
        let read = || -> ora_application::WorkflowRunPayload {
            let run = SqliteWorkflowRunRepository::new(f.pool.clone())
                .find_run(&f.run_id)
                .unwrap()
                .unwrap();
            serde_json::from_str(run.payload.as_deref().unwrap()).unwrap()
        };
        let before = read();
        SqliteWorkflowRunEngineRepository::new(f.pool.clone())
            .bind_node_run_session(&member.id, &SessionId::new("session-1"), 99)
            .unwrap();
        assert_eq!(before.iteration_ledger["iter"].len(), 1);
        let p = f.preview(x.as_ref(), 0);
        assert!(p.impact.stop.contains(&member.id.to_string()));
        let operation = f
            .coordinator
            .commit(f.commit_request(&p, "reset"))
            .await
            .unwrap();
        assert_eq!(operation.phase, WorkflowDebugPhase::Applied { revision: 1 });
        let after = read();
        assert!(!after.iteration_ledger.contains_key("iter"));
        assert!(after.variable_pool.revision > before.variable_pool.revision);
        complete(&member.id);
        assert_eq!(dispatched(&f.executor, "fix"), 2);
        assert!(
            node_runs(&f.pool, &f.run_id)
                .iter()
                .all(|r| r.node_id != "fix")
        );
        complete(&f.row("x").id);
        assert_eq!(f.row("fix").iteration, Some(0));
    });
}

/// Retiring a loop invalidates its active round, including late member completions.
#[test]
fn editing_upstream_cancels_the_old_loop_round() {
    run_test(async {
        use super::super::loop_resume_tests::LOOP_GRAPH;
        let mut graph: serde_json::Value = serde_json::from_str(LOOP_GRAPH).unwrap();
        let root: serde_json::Value = serde_json::from_str(GRAPH).unwrap();
        graph["nodes"]
            .as_array_mut()
            .unwrap()
            .push(root["nodes"][1].clone());
        graph["edges"][0]["target"] = json!("x");
        graph["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":"x","target":"loop"}));
        let f = Fixture::with_graph(Arc::new(NoSessions), &graph.to_string());
        let complete = |id: &WorkflowNodeRunId| {
            f.engine
                .complete_node(&f.run_id, id, Some("done".into()), None, None, vec![])
                .unwrap()
        };
        let x = f.row("x").id;
        complete(&x);
        let repo = SqliteWorkflowRunEngineRepository::new(f.pool.clone());
        let old_loop = f.row("loop").id;
        let old_round = repo.find_active_loop_round(&old_loop).unwrap().unwrap();
        let writer = repo
            .list_node_runs_in_scope(&old_round.id)
            .unwrap()
            .into_iter()
            .find(|row| row.node_id == "writer")
            .unwrap();
        assert_eq!(
            repo.bind_node_run_session(&writer.id, &SessionId::new("session-1"), 99)
                .unwrap(),
            ora_application::BindWorkflowNodeSessionResult::Bound
        );
        let p = f.preview(x.as_ref(), 0);
        assert!(p.impact.stop.contains(&writer.id.to_string()));
        let operation = f
            .coordinator
            .commit(f.commit_request(&p, "reset-loop"))
            .await
            .unwrap();
        assert_eq!(operation.phase, WorkflowDebugPhase::Applied { revision: 1 });
        let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
        let status: i64 = db
            .query_row(
                "SELECT status FROM workflow_execution_scopes WHERE id=?1",
                [old_round.id.as_ref()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status,
            ora_domain::WorkflowScopeStatus::Cancelled.database_value()
        );
        complete(&writer.id);
        assert_eq!(dispatched(&f.executor, "writer"), 1);
        complete(&f.row("x").id);
        let new_round = repo
            .find_active_loop_round(&f.row("loop").id)
            .unwrap()
            .unwrap();
        assert_ne!(new_round.id, old_round.id);
        assert_eq!(new_round.round_index, old_round.round_index);
        assert_eq!(dispatched(&f.executor, "writer"), 2);
    });
}
