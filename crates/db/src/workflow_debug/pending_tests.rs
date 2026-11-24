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
        .apply_after_stop("other", "continue", |_, _| Ok(()))
        .unwrap();
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
    assert_eq!(
        reopened.history("other").unwrap()[0].preview.target,
        preview.target
    );
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
