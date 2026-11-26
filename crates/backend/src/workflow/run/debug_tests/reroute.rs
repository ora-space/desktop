use super::*;
use ora_application::WorkflowRunPayload;
use pretty_assertions::assert_eq;

/// Both joins initially run through C. After C switches, only the join on the
/// independent Condition's selected port survives. The old foreach and its member retire.
#[test]
fn terminal_reroute_preserves_legacy_side_decision_and_drops_inactive_region() {
    run_test(async {
        let condition = |id| {
            json!({"id": id, "data": {"kind": "condition", "cases": [
                {"id": "yes", "logic": "and", "conditions": [
                    {"variableSelector": ["start", "flag"], "operator": "is", "value": true}
                ]}
            ]}})
        };
        let agent = |id| {
            json!({"id": id, "data": {"kind": "agent", "agentConfig": {
                "executor": {"agentCli": "c", "modelId": "m"}, "prompt": "work"
            }}})
        };
        let mut member = agent("member");
        member["parentId"] = json!("iter");
        let graph = json!({
            "nodes": [
                {"id": "start", "data": {"kind": "start", "inputVariables": [
                    {"name": "flag", "valueType": "boolean"}, {"name": "items", "valueType": "array[object]"}
                ]}},
                condition("c"), condition("side"), agent("kept_join"), agent("dropped_join"), agent("b"),
                {"id": "iter", "data": {"kind": "iteration", "iterationConfig": {
                    "iteratorSelector": ["start", "items"], "collectSelector": ["member", "output"],
                    "errorStrategy": "continue", "maxIterations": 5
                }}}, member
            ],
            "edges": [
                {"source": "start", "target": "c"}, {"source": "start", "target": "side"},
                {"source": "c", "sourceHandle": "yes", "target": "kept_join"},
                {"source": "c", "sourceHandle": "yes", "target": "dropped_join"},
                {"source": "c", "sourceHandle": "yes", "target": "iter"},
                {"source": "iter", "target": "member"},
                {"source": "c", "sourceHandle": "else", "target": "b"},
                {"source": "side", "sourceHandle": "yes", "target": "kept_join"},
                {"source": "side", "sourceHandle": "else", "target": "dropped_join"}
            ]
        });
        let inputs = [
            ("flag".into(), json!(true)),
            ("items".into(), json!([{"id": 1}])),
        ]
        .into();
        let f = Fixture::with_inputs(Arc::new(NoSessions), &graph.to_string(), inputs);
        let complete = |id: &str, value: &str| {
            f.engine
                .complete_node(
                    &f.run_id,
                    &f.row(id).id,
                    Some(value.into()),
                    None,
                    None,
                    vec![],
                )
                .unwrap();
        };
        for id in ["kept_join", "dropped_join", "member"] {
            complete(id, "old");
        }
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        let side_attempt = f.row("side").id;

        // Persist the historical representation of the side decision, then enter
        // through the normal preview and commit APIs as a reopened run would.
        let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
        let raw: String = db
            .query_row(
                "SELECT payload FROM workflow_runs WHERE id=?1",
                [f.run_id.as_ref()],
                |r| r.get(0),
            )
            .unwrap();
        let mut payload: WorkflowRunPayload = serde_json::from_str(&raw).unwrap();
        let selected = payload.condition_decisions.remove("side").unwrap();
        payload
            .variable_pool
            .declare("side.selected_branch_id", "string", "side");
        payload
            .variable_pool
            .set("side.selected_branch_id", "side", json!(selected))
            .unwrap();
        db.execute(
            "UPDATE workflow_runs SET payload=?1 WHERE id=?2",
            (serde_json::to_string(&payload).unwrap(), f.run_id.as_ref()),
        )
        .unwrap();

        let preview = f.coordinator.preview(serde_json::from_value(json!({
            "runId": f.run_id, "nodeRunId": f.row("c").id, "baseRevision": 0,
            "patch": [{"op": "replace", "path": "/cases/0/conditions/0/value", "value": false}]
        })).unwrap()).unwrap();
        assert_eq!(preview.impact.schedule, ["c", "b", "kept_join"]);
        for id in ["kept_join", "dropped_join", "iter", "member"] {
            assert!(
                preview
                    .impact
                    .invalidate
                    .contains(&f.row(id).id.to_string()),
                "{id}"
            );
        }
        let operation = f
            .coordinator
            .commit(f.commit_request(&preview, "reroute"))
            .await
            .unwrap();
        assert_eq!(operation.phase, WorkflowDebugPhase::Applied { revision: 1 });
        complete("b", "new branch");
        assert_eq!(f.run_status(), WorkflowRunStatus::Running);
        complete("kept_join", "new join");
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(f.row("side").id, side_attempt);
        assert_eq!(f.row("kept_join").output.as_deref(), Some("new join"));
        assert_eq!(
            ["kept_join", "dropped_join", "member", "b"].map(|id| dispatched(&f.executor, id)),
            [2, 1, 1, 1]
        );
    });
}
