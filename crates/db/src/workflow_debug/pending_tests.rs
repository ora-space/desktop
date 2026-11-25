use super::*;
use pretty_assertions::assert_eq;

/// Exercises the ADR's root pending target, including an independent sibling advancing between requests.
#[test]
fn pending_edit_binds_first_dispatch_without_allocating_an_attempt() {
    let (_temp, path, store) = fixture();
    let graph = json!({"nodes":[
        {"id":"start","data":{"kind":"start","inputVariables":[{"name":"text","valueType":"string"}]}},
        {"id":"agent","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"cli","modelId":"m"},"prompt":"old"}}},
        {"id":"sib","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"cli","modelId":"m"},"prompt":"independent"}}}],
        "edges":[{"source":"start","target":"agent"},{"source":"start","target":"sib"}]});
    let mut wire = json!({"runId":"other","baseRevision":0,"pendingTarget":{"rootScopeId":"root:other","scopeId":"root:other","nodeId":"agent"},"patch":[{"op":"replace","path":"/agentConfig/prompt","value":"echo {{#start.text#}}"}]});
    let request: PreviewWorkflowDebugRequest = serde_json::from_value(wire.clone()).unwrap();
    wire["nodeRunId"] = json!("t3");
    assert!(serde_json::from_value::<PreviewWorkflowDebugRequest>(wire).is_err());
    store
        .pool
        .with_connection(|db| bind_graph(db, graph))
        .unwrap();
    assert!(
        matches!(store.preview(&request,impact),Err(DatabaseError::WorkflowDebug(DebugRejection::MissingFact(reason))) if reason.contains("start"))
    );
    let mut payload = ora_application::WorkflowRunPayload::default();
    payload
        .variable_pool
        .declare("start.text", "string", "start");
    payload.variable_pool.declare("sib.result", "string", "sib");
    store
        .pool
        .with_connection(|db| {
            insert_attempt(db, "start-ok", "start", "start", None)?;
            db.execute(
                "UPDATE workflow_node_runs SET status=2 WHERE id='start-ok'",
                [],
            )?;
            db.execute(
                "UPDATE workflow_runs SET payload=?1 WHERE id='other'",
                [serde_json::to_string(&payload)?],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(store.preview(&request, impact).is_err());
    payload
        .variable_pool
        .set("start.text", "start", json!("hello"))
        .unwrap();
    let save = |p: &ora_application::WorkflowRunPayload| {
        store
            .pool
            .with_connection(|db| {
                db.execute(
                    "UPDATE workflow_runs SET payload=?1 WHERE id='other'",
                    [serde_json::to_string(p)?],
                )?;
                Ok(())
            })
            .unwrap()
    };
    save(&payload);
    let preview = store.preview(&request, impact).unwrap();
    assert!(preview.target.node_run_id.is_none());
    let commit = |id: String| CommitWorkflowDebugRequest {
        run_id: "other".into(),
        request_id: "continue".into(),
        preview_id: id,
    };
    // A relevant value disappearing invalidates the first receipt.
    let mut missing = payload.clone();
    missing.variable_pool.values.remove("start.text");
    save(&missing);
    assert!(matches!(
        store.claim(&commit(preview.preview_id)),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
    save(&payload);
    let preview = store.preview(&request, impact).unwrap();
    store
        .pool
        .with_connection(|db| insert_attempt(db, "s1", "sib", "agent", None))
        .unwrap();
    payload
        .variable_pool
        .set("sib.result", "sib", json!("done"))
        .unwrap();
    save(&payload);
    // This edit reads the sibling through a template even though no edge connects them.
    store
        .pool
        .with_connection(|db| {
            db.execute("UPDATE workflow_node_runs SET status=2 WHERE id='s1'", [])?;
            Ok(())
        })
        .unwrap();
    let mut dependent = request.clone();
    dependent.patch = vec![WorkflowDebugPatch::Replace {
        path: "/agentConfig/prompt".into(),
        value: json!("{{#sib.result#}}"),
    }];
    let dependent = store.preview(&dependent, impact).unwrap();
    payload
        .variable_pool
        .set("sib.result", "sib", json!("changed"))
        .unwrap();
    save(&payload);
    assert!(matches!(
        store.claim(&commit(dependent.preview_id)),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
    let commit = commit(preview.preview_id);
    store.claim(&commit).unwrap();
    store
        .apply_after_stop("other", "continue", 1, |_, _| Ok(()))
        .unwrap();
    let first_dispatch =
        |store: &WorkflowDebugStore| store.history("other").unwrap()[0].first_node_run_id.clone();
    // Nothing is allocated by the confirmation itself, and another node starting first does not count.
    assert_eq!(first_dispatch(&store), None);
    store
        .pool
        .with_connection(|db| {
            db.execute(
                "UPDATE workflow_node_runs SET is_deleted=1 WHERE id='s1'",
                [],
            )?;
            insert_attempt(db, "s2", "sib", "agent", None)
        })
        .unwrap();
    assert_eq!(first_dispatch(&store), None);
    store
        .pool
        .with_connection(|db| {
            let count: u32 = db.query_row(
                "SELECT COUNT(*) FROM workflow_node_runs WHERE run_id='other' AND node_id='agent'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(count, 0);
            insert_attempt(db, "first", "agent", "agent", None)
        })
        .unwrap();
    let reopened = WorkflowDebugStore::new(RepositoryPool::new(&path).unwrap());
    assert_eq!(first_dispatch(&reopened), Some("first".into()));
    let first = reopened
        .attempt(&GetWorkflowDebugAttemptRequest {
            run_id: "other".into(),
            node_run_id: "first".into(),
        })
        .unwrap();
    assert_eq!(first.revision, Some(1));
    assert_eq!(
        first.configuration.unwrap()["agentConfig"]["prompt"],
        "echo {{#start.text#}}"
    );
    let history = reopened.history("other").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        (
            history[0].preview.target.clone(),
            history[0].first_node_run_id.as_deref()
        ),
        (preview.target, Some("first"))
    );
    // A replacement attempt of the same position never takes the binding over.
    reopened
        .pool
        .with_connection(|db| {
            db.execute(
                "UPDATE workflow_node_runs SET is_deleted=1 WHERE id='first'",
                [],
            )?;
            insert_attempt(db, "again", "agent", "agent", None)
        })
        .unwrap();
    assert_eq!(first_dispatch(&reopened), Some("first".into()));
}

/// A target admitted by the normal scheduler after preview must receive no edit or stop request.
#[test]
fn first_scheduler_claim_invalidates_logical_preview() {
    let (_temp, _, store) = fixture();
    store
        .pool
        .with_connection(|db| {
            insert_attempt(db, "start-ok", "start", "start", None)?;
            db.execute(
                "UPDATE workflow_node_runs SET status=2 WHERE id='start-ok'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let mut request = edit("new", 0);
    request.run_id = "other".into();
    request.selection = WorkflowDebugSelection::PendingTarget(PendingWorkflowDebugTarget {
        root_scope_id: "root:other".into(),
        scope_id: "root:other".into(),
        node_id: "agent".into(),
        index: None,
        container_node_run_id: None,
    });
    let preview = store.preview(&request, impact).unwrap();
    store
        .pool
        .with_connection(|db| insert_attempt(db, "winner", "agent", "agent", None))
        .unwrap();
    assert!(matches!(
        store.claim(&CommitWorkflowDebugRequest {
            run_id: "other".into(),
            request_id: "late".into(),
            preview_id: preview.preview_id
        }),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
    assert!(store.history("other").unwrap().is_empty());
}

/// Uses a frozen container input even when its mutable root selector has moved on.
#[test]
fn pending_foreach_member_requires_owner_and_exact_bound_item() {
    let (_temp, _, store) = fixture();
    let graph = json!({"nodes":[
        {"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"}]}},
        {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","items"],"collectSelector":["agent","output"]}}},
        {"id":"agent","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"cli","modelId":"m"},"prompt":"old"}}}],
        "edges":[{"source":"start","target":"iter"},{"source":"iter","target":"agent"}]});
    let mut payload = ora_application::WorkflowRunPayload::default();
    payload
        .variable_pool
        .declare("start.items", "array[string]", "start");
    payload
        .variable_pool
        .set("start.items", "start", json!(["a", "b", "c"]))
        .unwrap();
    store
        .pool
        .with_connection(|db| {
            bind_graph(db, graph)?;
            db.execute(
                "UPDATE workflow_runs SET payload=?1 WHERE id='other'",
                [serde_json::to_string(&payload)?],
            )?;
            insert_attempt(db, "it", "iter", "iteration", None)?;
            payload
                .variable_pool
                .declare("iter.index", "number", "iter");
            payload.variable_pool.declare("iter.item", "string", "iter");
            payload
                .variable_pool
                .set("iter.index", "iter", json!(2))
                .unwrap();
            payload
                .variable_pool
                .set("iter.item", "iter", json!("c"))
                .unwrap();
            payload
                .variable_pool
                .set("start.items", "start", json!(["x", "y", "z"]))
                .unwrap();
            db.execute(
                "UPDATE workflow_runs SET payload=?1 WHERE id='other'",
                [serde_json::to_string(&payload)?],
            )?;
            Ok(())
        })
        .unwrap();
    let request = |owner: Option<&str>, index: Option<u32>| {
        let mut req = edit("{{#iter.item#}}", 0);
        req.run_id = "other".into();
        req.selection = WorkflowDebugSelection::PendingTarget(PendingWorkflowDebugTarget {
            root_scope_id: "root:other".into(),
            scope_id: "root:other".into(),
            node_id: "agent".into(),
            container_node_run_id: owner.map(String::from),
            index,
        });
        req
    };
    assert!(
        matches!(store.preview(&request(None,Some(2)),impact), Err(DatabaseError::WorkflowDebug(DebugRejection::MissingFact(reason))) if reason.contains("containerNodeRunId"))
    );
    assert!(
        store
            .preview(&request(Some("it"), Some(1)), impact)
            .is_err()
    );
    let preview = store
        .preview(&request(Some("it"), Some(2)), |input| {
            assert_eq!(input.pool.values["iter.item"], "c");
            impact(input)
        })
        .unwrap();
    assert_eq!(preview.target.index, Some(2));
    assert_eq!(preview.target.container_node_run_id.as_deref(), Some("it"));
    store
        .claim(&CommitWorkflowDebugRequest {
            run_id: "other".into(),
            preview_id: preview.preview_id,
            request_id: "item".into(),
        })
        .unwrap();
    store
        .apply_after_stop("other", "item", 1, |_, _| Ok(()))
        .unwrap();
    store
        .pool
        .with_connection(|db| insert_attempt(db, "wrong-index", "agent", "agent", Some(1)))
        .unwrap();
    assert_eq!(store.history("other").unwrap()[0].first_node_run_id, None);
    store
        .pool
        .with_connection(|db| insert_attempt(db, "selected-index", "agent", "agent", Some(2)))
        .unwrap();
    assert_eq!(
        store.history("other").unwrap()[0]
            .first_node_run_id
            .as_deref(),
        Some("selected-index")
    );
}

/// A Loop member uses the actual round scope and carried value, never a future round.
#[test]
fn pending_loop_member_uses_its_live_round() {
    let (_temp, _, store) = fixture();
    let graph = json!({"schemaVersion":2,"nodes":[{"id":"start","data":{"kind":"start"}},{"id":"loop","data":{"kind":"loop","loopConfig":{
        "maxIterations":5,"variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":""},"feedback":["writer","output"]}],
        "until":{"logic":"and","conditions":[{"variableSelector":["writer","output"],"operator":"equals","value":"done"}]},
        "outputs":[{"name":"result","variableSelector":["writer","output"]}]}}},
        {"id":"entry","data":{"kind":"start","containerId":"loop"}},
        {"id":"writer","data":{"kind":"agent","containerId":"loop","agentConfig":{"executor":{"agentCli":"cli","modelId":"m"},"prompt":"old"}}}],
        "edges":[{"source":"start","target":"loop"},{"source":"entry","target":"writer"}]});
    let mut pool = WorkflowVariablePool::default();
    pool.declare("loop.draft", "string", "loop");
    pool.set("loop.draft", "loop", json!("third")).unwrap();
    store.pool.with_connection(|db|{
        bind_graph(db,graph)?;
        insert_attempt(db,"lp","loop","loop",None)?;
        db.execute("INSERT INTO workflow_execution_scopes(id,run_id,parent_loop_node_run_id,round_index,status,state,created_at,updated_at)
            VALUES('round3','other','lp',3,1,?1,1,1)",[json!({"variablePool":pool,"conditionDecisions":{}}).to_string()])?;
        db.execute("INSERT INTO workflow_node_runs(id,run_id,scope_id,node_id,node_type,status,created_at,updated_at)
            VALUES('entry3','other','round3','entry','start',2,1,1)",[])?;
        Ok(())
    }).unwrap();
    let request = |scope: &str| {
        serde_json::from_value(json!({"runId":"other","baseRevision":0,
        "pendingTarget":{"rootScopeId":"root:other","scopeId":scope,"nodeId":"writer"},"patch":[{"op":"replace","path":"/agentConfig/prompt","value":"{{#loop.draft#}}"}]})).unwrap()
    };
    assert!(store.preview(&request("round4"), impact).is_err());
    let preview = store
        .preview(&request("round3"), |input| {
            assert_eq!(input.pool.values["loop.draft"], "third");
            impact(input)
        })
        .unwrap();
    assert_eq!(preview.target.round, Some(3));
    assert_eq!(preview.target.container_node_run_id.as_deref(), Some("lp"));
}
