use super::*;
use crate::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
use pretty_assertions::assert_eq;
use serde_json::json;

/// Seeds a real migrated database with two runs of the same published definition.
fn fixture() -> (tempfile::TempDir, DatabaseLocation, WorkflowDebugStore) {
    let temp = tempfile::tempdir().unwrap();
    let location = DatabaseLocation::path(temp.path().join("debug.sqlite"));
    let pool = DatabaseBootstrapper::system()
        .bootstrap_repository_pool(&location, &default_migration_catalog().unwrap())
        .unwrap();
    pool.with_connection(|db| {
        db.execute_batch("INSERT INTO projects(id,name,created_at,updated_at) VALUES ('p','p',1,1);
            INSERT INTO workspace_locations(id,location_kind,locator_json,created_at,updated_at) VALUES ('l','local_filesystem','{}',1,1);
            INSERT INTO workspaces(id,project_id,workspace_kind,location_id,lifecycle,created_at,updated_at) VALUES ('w','p','main','l','active',1,1);
            INSERT INTO workflows(id,name,created_at,updated_at) VALUES ('wf','wf',1,1);")?;
        let graph = json!({"nodes":[{"id":"start","data":{"kind":"start"}},
            {"id":"agent","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"old","modelId":"m"},"prompt":"old"}}}],
            "edges":[{"source":"start","target":"agent"}]});
        db.execute("INSERT INTO workflow_snapshots(id,workflow_id,version,graph,created_at) VALUES('snap','wf','1',?1,1)",[graph.to_string()])?;
        for id in ["F","other"] {
            db.execute("INSERT INTO workflow_runs(id,workspace_id,workflow_id,snapshot_id,name,run_status,created_at,updated_at) VALUES(?1,'w','wf','snap','run',1,1,1)",[id])?;
        }
        db.execute("INSERT INTO workflow_node_runs(id,run_id,scope_id,node_id,node_type,status,created_at,updated_at)
            SELECT 't3','F',scope_id,'agent','agent',1,1,1 FROM workflow_run_root_scopes WHERE run_id='F'",[])?;
        Ok(())
    }).unwrap();
    (temp, location, WorkflowDebugStore::new(pool))
}

/// A fixture strategy echoes the physical target; actual stop/DAG/region strategies own planning.
fn impact(input: &DebugPlanningInput) -> Result<WorkflowDebugImpact, DebugRejection> {
    Ok(WorkflowDebugImpact {
        keep: vec![],
        invalidate: vec![input.target.node_run_id.clone()],
        stop: vec![],
        schedule: vec![input.target.node_id.clone()],
        predicted_branch: None,
    })
}

/// Edits a prompt using the actual wire contract.
fn edit(value: &str, revision: u32) -> PreviewWorkflowDebugRequest {
    serde_json::from_value(
        json!({"runId":"F","nodeRunId":"t3","baseRevision":revision,"patch":[
        {"op":"replace","path":"/agentConfig/prompt","value":value}]}),
    )
    .unwrap()
}

/// Replays a lost response after reopening and checks CAS, rollback and request isolation.
#[test]
fn revision_history_survives_reopen_and_two_edits() {
    let (_temp, path, store) = fixture();
    let missing = store.preview(&edit("one", 0), impact);
    assert!(matches!(
        missing,
        Err(DatabaseError::WorkflowDebug(
            DebugRejection::MissingInputEvidence
        ))
    ));
    store
        .record_agent_dispatch(
            "F",
            "t3",
            &json!({"variablePool":WorkflowVariablePool::default()}),
        )
        .unwrap();
    assert!(matches!(
        store.preview(&edit("{{#missing.output#}}", 0), impact),
        Err(DatabaseError::WorkflowDebug(
            DebugRejection::InvalidPatch { .. }
        ))
    ));
    let preview = store.preview(&edit("one", 0), impact).unwrap();
    let mut commit = CommitWorkflowDebugRequest {
        run_id: "F".into(),
        preview_id: preview.preview_id.clone(),
        request_id: "k".into(),
    };
    store
        .pool
        .with_connection(|db| {
            Ok(db
                .execute("UPDATE workflow_node_runs SET status=2 WHERE id='t3'", [])
                .map(|_| ())?)
        })
        .unwrap();
    assert!(matches!(
        store.claim(&commit),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
    let fresh = store.preview(&edit("one", 0), impact).unwrap();
    commit.preview_id = fresh.preview_id.clone();
    assert!(matches!(
        store.claim(&commit).unwrap(),
        DebugClaim::Claimed(_)
    ));
    assert!(matches!(
        store.claim(&commit).unwrap(),
        DebugClaim::Existing(_)
    ));
    assert!(
        store
            .apply_after_stop("F", "k", |_, _| Err(
                DatabaseError::IncompleteWorkflowRunContext
            ))
            .is_err()
    );
    assert_eq!(
        store.history("F").unwrap()[0].operation.phase,
        WorkflowDebugPhase::Stopping
    );
    let applied = store.apply_after_stop("F", "k", |_, _| Ok(())).unwrap();
    assert_eq!(applied.phase, WorkflowDebugPhase::Applied { revision: 1 });
    let reopened = WorkflowDebugStore::new(RepositoryPool::new(&path).unwrap());
    let DebugClaim::Existing(original) = reopened.claim(&commit).unwrap() else {
        panic!("duplicate claim");
    };
    assert_eq!(original.operation, applied);
    let next = reopened.preview(&edit("two", 1), impact).unwrap();
    commit.preview_id = next.preview_id;
    assert!(matches!(
        reopened.claim(&commit),
        Err(DatabaseError::WorkflowDebug(
            DebugRejection::RequestConflict
        ))
    ));
    commit.request_id = "second".into();
    reopened.claim(&commit).unwrap();
    reopened
        .apply_after_stop("F", "second", |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        reopened
            .history("F")
            .unwrap()
            .iter()
            .map(|e| e.operation.phase.clone())
            .collect::<Vec<_>>(),
        vec![
            WorkflowDebugPhase::Applied { revision: 1 },
            WorkflowDebugPhase::Applied { revision: 2 }
        ]
    );
    assert!(reopened.history("other").unwrap().is_empty());
    reopened.pool.with_connection(|db| {
        let prompt:String=db.query_row("SELECT json_extract(graph,'$.nodes[1].data.agentConfig.prompt') FROM workflow_snapshots WHERE id='snap'",[],|r|r.get(0))?;
        assert_eq!(prompt,"old"); Ok(())
    }).unwrap();
}

/// Creates real rows using the same scope and iteration columns as the scheduler.
fn insert_attempt(
    db: &Connection,
    id: &str,
    node: &str,
    kind: &str,
    index: Option<u32>,
) -> Result<(), DatabaseError> {
    db.execute("INSERT INTO workflow_node_runs(id,run_id,scope_id,node_id,node_type,iteration,status,created_at,updated_at)
        SELECT ?1,'other',scope_id,?2,?3,?4,1,1,1 FROM workflow_run_root_scopes WHERE run_id='other'",
        params![id,node,kind,index])?;
    Ok(())
}

/// Binds a fresh snapshot before the fixture run starts any nodes.
fn bind_graph(db: &Connection, graph: Value) -> Result<(), DatabaseError> {
    db.execute("INSERT INTO workflow_snapshots(id,workflow_id,version,graph,created_at) VALUES('selected','wf','2',?1,1)",[graph.to_string()])?;
    db.execute(
        "UPDATE workflow_runs SET snapshot_id='selected' WHERE id='other'",
        [],
    )?;
    Ok(())
}

/// The fifth item's live selector may change without changing the third attempt's input.
#[test]
fn item_three_uses_its_own_input_and_revision_suffix() {
    let (_temp, _path, store) = fixture();
    let graph = json!({"nodes":[
        {"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"}]}},
        {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","items"],"collectSelector":["agent","output"]}}},
        {"id":"agent","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"cli","modelId":"m"},"prompt":"{{#iter.item#}}"}}},
        {"id":"out","data":{"kind":"output","outputs":[{"name":"result","variableSelector":["iter","output"]}]}}],
        "edges":[{"source":"start","target":"iter"},{"source":"iter","target":"agent"},{"source":"iter","target":"out"}]});
    store
        .pool
        .with_connection(|db| {
            bind_graph(db, graph)?;
            insert_attempt(db, "it1", "iter", "iteration", /*index*/ None)?;
            Ok(())
        })
        .unwrap();
    for (i, item) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
        let mut pool = WorkflowVariablePool::default();
        pool.declare("iter.item", "string", "iter");
        pool.set("iter.item", "iter", json!(item)).unwrap();
        let id = format!("f{i}");
        store
            .pool
            .with_connection(|db| {
                let payload = ora_application::WorkflowRunPayload {
                    variable_pool: pool.clone(),
                    ..Default::default()
                };
                db.execute(
                    "UPDATE workflow_runs SET payload=?1 WHERE id='other'",
                    [serde_json::to_string(&payload)?],
                )?;
                insert_attempt(db, &id, "agent", "agent", Some(i as u32))
            })
            .unwrap();
        store
            .record_agent_dispatch("other", &id, &json!({"variablePool":pool}))
            .unwrap();
        store
            .pool
            .with_connection(|db| {
                db.execute(
                    "UPDATE workflow_node_runs SET status=2,output=?2 WHERE id=?1",
                    params![id, item],
                )?;
                Ok(())
            })
            .unwrap();
    }
    store
        .pool
        .with_connection(|db| insert_attempt(db, "out-before", "out", "output", /*index*/ None))
        .unwrap();
    let mut request = edit("new-{{#iter.item#}}", 0);
    request.run_id = "other".into();
    request.node_run_id = "f2".into();
    let p = store
        .preview(&request, |input| {
            assert_eq!(
                (&input.target.container_node_run_id, input.target.index),
                (&Some("it1".into()), Some(2))
            );
            assert_eq!(input.pool.values["iter.item"], json!("c"));
            assert_eq!(
                input.current_payload.as_ref().unwrap().variable_pool.values["iter.item"],
                json!("e")
            );
            let prompt = &input
                .candidate
                .execution_node("agent")
                .unwrap()
                .agent_config
                .as_ref()
                .unwrap()
                .prompt;
            assert_eq!(
                ora_application::render_variable_template(prompt, &input.pool).unwrap(),
                "new-c"
            );
            Ok(WorkflowDebugImpact {
                keep: vec!["f0".into(), "f1".into()],
                invalidate: vec!["f2".into(), "f3".into(), "f4".into()],
                stop: vec![],
                schedule: vec!["it1[2..4]".into()],
                predicted_branch: None,
            })
        })
        .unwrap();
    let commit = CommitWorkflowDebugRequest {
        run_id: "other".into(),
        preview_id: p.preview_id,
        request_id: "confirm".into(),
    };
    store.claim(&commit).unwrap();
    store
        .apply_after_stop("other", "confirm", |tx, plan| {
            for id in &plan.impact.invalidate {
                tx.execute(
                    "UPDATE workflow_node_runs SET is_deleted=1 WHERE id=?1",
                    [id],
                )?;
            }
            insert_attempt(tx, "new3", "agent", "agent", Some(2))
        })
        .unwrap();
    let attempt = |id: &str| {
        store
            .attempt(&GetWorkflowDebugAttemptRequest {
                run_id: "other".into(),
                node_run_id: id.into(),
            })
            .unwrap()
    };
    let output = attempt("out-before");
    assert_eq!(output.revision, Some(0));
    assert_eq!(output.configuration.unwrap()["kind"], "output");
    let old = attempt("f2");
    assert_eq!(
        old.retired_by_operation_id,
        Some(
            store.history("other").unwrap()[0]
                .operation
                .operation_id
                .clone()
        )
    );
    assert_eq!(
        (
            old.superseded,
            old.revision,
            old.configuration.unwrap()["agentConfig"]["prompt"].clone()
        ),
        (true, Some(0), json!("{{#iter.item#}}"))
    );
    assert_eq!(attempt("f1").revision, Some(0));
    assert_eq!(
        attempt("new3").configuration.unwrap()["agentConfig"]["prompt"],
        json!("new-{{#iter.item#}}")
    );
    assert!(matches!(
        store.preview(&request, impact),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
}

/// Pins independent clients racing for the same preview and the failure/release lifecycle.
#[test]
fn double_click_has_one_owner_and_failed_stop_stays_fenced() {
    let (_temp, _path, store) = fixture();
    store
        .record_agent_dispatch(
            "F",
            "t3",
            &json!({"variablePool":WorkflowVariablePool::default()}),
        )
        .unwrap();
    let p = store.preview(&edit("one", 0), impact).unwrap();
    let commit = CommitWorkflowDebugRequest {
        run_id: "F".into(),
        preview_id: p.preview_id,
        request_id: "key".into(),
    };
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|s| {
        let handles = (0..2)
            .map(|_| {
                s.spawn(|| {
                    barrier.wait();
                    matches!(store.claim(&commit).unwrap(), DebugClaim::Claimed(_))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| usize::from(h.join().unwrap()))
                .sum::<usize>(),
            1
        );
    });
    store
        .record_stop_failure("F", "key", "still running")
        .unwrap();
    assert!(matches!(
        store.preview(&edit("next", 0), impact),
        Err(DatabaseError::WorkflowDebug(
            DebugRejection::OperationInProgress
        ))
    ));
    let DebugClaim::Existing(failed) = store.claim(&commit).unwrap() else {
        panic!("replayed stop");
    };
    assert_eq!(
        failed.operation.phase,
        WorkflowDebugPhase::StopFailed {
            reason: "still running".into()
        }
    );
    store.release_failed_stop("F", "key").unwrap();
    assert!(store.preview(&edit("next", 0), impact).is_ok());
}

/// The frozen boolean selects ELSE after replacing a comparison, without changing case identity.
#[test]
fn condition_preview_uses_the_observed_input_and_preserves_the_old_decision() {
    let (_temp, _path, store) = fixture();
    let branch = |name: &str| json!({"id":name,"data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"cli","modelId":"m"},"prompt":name}}});
    let graph = json!({"nodes":[{"id":"start","data":{"kind":"start","inputVariables":[{"name":"flag","valueType":"boolean"}]}},
        {"id":"condition","data":{"kind":"condition","cases":[{"id":"A","logic":"and","conditions":[
            {"variableSelector":["start","flag"],"operator":"is","value":true}]}]}},branch("A"),branch("B")],
        "edges":[{"source":"start","target":"condition"},{"source":"condition","sourceHandle":"A","target":"A"},
            {"source":"condition","sourceHandle":"else","target":"B"}]});
    store
        .pool
        .with_connection(|db| {
            bind_graph(db, graph)?;
            insert_attempt(db, "c1", "condition", "condition", /*index*/ None)?;
            let mut pool = WorkflowVariablePool::default();
            pool.declare("start.flag", "boolean", "start");
            pool.set("start.flag", "start", json!(true)).unwrap();
            db.execute(
                "UPDATE workflow_node_runs SET status=2,payload=?1 WHERE id='c1'",
                [json!({"debugCondition":{"branch":"A","variablePool":pool}}).to_string()],
            )?;
            Ok(())
        })
        .unwrap();
    let mut request: PreviewWorkflowDebugRequest =
        serde_json::from_value(json!({"runId":"other","nodeRunId":"c1","baseRevision":0,
        "patch":[{"op":"replace","path":"/cases/0/conditions/0/value","value":false}]}))
        .unwrap();
    let preview = store
        .preview(&request, |input| {
            let mut result = impact(input)?;
            result.predicted_branch = input.predicted_branch.clone();
            Ok(result)
        })
        .unwrap();
    assert_eq!(preview.impact.predicted_branch.as_deref(), Some("else"));
    assert_eq!(
        store
            .attempt(&GetWorkflowDebugAttemptRequest {
                run_id: "other".into(),
                node_run_id: "c1".into()
            })
            .unwrap()
            .selected_branch
            .as_deref(),
        Some("A")
    );
    request.patch = vec![WorkflowDebugPatch::Replace {
        path: "/cases/0/id".into(),
        value: json!("renamed"),
    }];
    assert!(matches!(
        store.preview(&request, impact),
        Err(DatabaseError::WorkflowDebug(
            DebugRejection::InvalidPatch { .. }
        ))
    ));
}
