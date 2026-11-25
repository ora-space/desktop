use super::*;
use crate::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
use ora_application::WorkflowRunEngineRepository;
use pretty_assertions::assert_eq;
use serde_json::json;

#[path = "pending_tests.rs"]
mod pending_tests;
#[path = "storage_measurement.rs"]
mod storage_measurement;

/// Backoff is a real attempt with its own entry, but no invented dispatch. The timer racing
/// confirmation invalidates the receipt even though the physical attempt ID does not change.
#[test]
fn waiting_retry_uses_its_predecessor_until_the_timer_starts_it() {
    use ora_application::{
        NodeFailure, NodeFailureKind, NodeRetryToSchedule, WorkflowRetryRepository,
    };
    let (_temp, _, store) = fixture();
    let input =
        json!({"variablePool":WorkflowVariablePool::default(), "prompt":"original dispatch"});
    assert!(store.record_agent_dispatch("F", "t3", &input).unwrap());
    assert!(!store.record_agent_dispatch("F", "t3", &json!({})).unwrap());
    let repo = crate::SqliteWorkflowRunEngineRepository::new(store.pool.clone());
    let waiting = ora_domain::WorkflowNodeRunId::new("retry");
    repo.schedule_node_retry(
        &ora_domain::WorkflowNodeRunId::new("t3"),
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
    let mut request = edit("corrected", 0);
    request.selection = WorkflowDebugSelection::NodeRunId(waiting.to_string());
    let p = store
        .preview(&request, |i| {
            assert_eq!(i.dispatch.as_ref(), Some(&input));
            assert_eq!(i.target.node_run_id.as_deref(), Some("retry"));
            assert_eq!(i.entry["scopeId"], "root:F");
            impact(i)
        })
        .unwrap();
    assert!(
        !store
            .attempt(&GetWorkflowDebugAttemptRequest {
                run_id: "F".into(),
                node_run_id: waiting.to_string()
            })
            .unwrap()
            .input_recorded
    );
    assert_eq!(p.input_source_node_run_id.as_deref(), Some("t3"));
    repo.begin_node_retry(&waiting, 30).unwrap();
    assert!(matches!(
        store.claim(&CommitWorkflowDebugRequest {
            run_id: "F".into(),
            preview_id: p.preview_id,
            request_id: "confirm".into()
        }),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
    assert!(store.preview(&request, impact).is_err());
    // This retry failed before recording a dispatch. The next wait can prove its input from t3.
    let second = ora_domain::WorkflowNodeRunId::new("second");
    repo.schedule_node_retry(
        &waiting,
        &NodeFailure::new(NodeFailureKind::Session, "startup"),
        &NodeRetryToSchedule {
            node_run_id: second.clone(),
            retry: 2,
            max_retries: 2,
            delay_ms: 20,
            due_at: 60,
        },
        40,
    )
    .unwrap();
    request.selection = WorkflowDebugSelection::NodeRunId(second.to_string());
    assert_eq!(
        store
            .preview(&request, impact)
            .unwrap()
            .input_source_node_run_id
            .as_deref(),
        Some("t3")
    );
    for invalid in [
        "revision=1",
        "container_attempt='retry'",
        "snapshot_id='other-snapshot'",
    ] {
        store.pool.with_connection(|db| {
            db.execute("INSERT OR IGNORE INTO workflow_snapshots(id,workflow_id,version,graph,created_at) SELECT 'other-snapshot',workflow_id,'2',graph,1 FROM workflow_snapshots WHERE id='snap'", [])?;
            db.execute(&format!("UPDATE workflow_debug_attempts SET {invalid} WHERE node_run_id='t3'"), [])?;
            Ok(())
        }).unwrap();
        assert!(matches!(
            store.preview(&request, impact),
            Err(DatabaseError::WorkflowDebug(
                DebugRejection::MissingInputEvidence
            ))
        ));
        store.pool.with_connection(|db| {
            db.execute("UPDATE workflow_debug_attempts SET revision=0,container_attempt=NULL,snapshot_id='snap' WHERE node_run_id='t3'", [])?;
            Ok(())
        }).unwrap();
    }
}

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
        invalidate: input.target.node_run_id.clone().into_iter().collect(),
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

/// Admission and apply must independently reject a stale or unavailable target.
#[test]
fn invalid_run_or_replaced_attempt_cannot_apply() {
    let (_temp, _, store) = fixture();
    store
        .record_agent_dispatch(
            "F",
            "t3",
            &json!({"variablePool":WorkflowVariablePool::default()}),
        )
        .unwrap();
    let p = store.preview(&edit("new", 0), impact).unwrap();
    let mut request = CommitWorkflowDebugRequest {
        run_id: "F".into(),
        preview_id: p.preview_id,
        request_id: "  ".into(),
    };
    assert!(matches!(
        store.claim(&request),
        Err(DatabaseError::WorkflowDebug(DebugRejection::EmptyRequestId))
    ));
    request.request_id = "k".into();
    store.claim(&request).unwrap();
    store
        .pool
        .with_connection(|db| {
            db.execute(
                "UPDATE workflow_node_runs SET is_deleted=1 WHERE id='t3'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        store.apply_after_stop("F", "k", 4, |_, _| panic!("stale settlement")),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
    assert_eq!(
        store.history("F").unwrap()[0].operation.phase,
        WorkflowDebugPhase::Stopping
    );
    let mut other = edit("test", 0);
    other.run_id = "other".into();
    other.selection = WorkflowDebugSelection::NodeRunId("other-agent".into());
    store
        .pool
        .with_connection(|db| insert_attempt(db, "other-agent", "agent", "agent", None))
        .unwrap();
    store
        .record_agent_dispatch(
            "other",
            "other-agent",
            &json!({"variablePool":WorkflowVariablePool::default()}),
        )
        .unwrap();
    for (status, deleted) in [(0, 0), (1, 1)] {
        store
            .pool
            .with_connection(|db| {
                db.execute(
                    "UPDATE workflow_runs SET run_status=?1,is_deleted=?2 WHERE id='other'",
                    params![status, deleted],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            store.preview(&other, impact),
            Err(DatabaseError::WorkflowDebug(DebugRejection::InvalidTarget))
        ));
    }
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
            .apply_after_stop("F", "k", 1, |_, _| Err(
                DatabaseError::IncompleteWorkflowRunContext
            ))
            .is_err()
    );
    assert_eq!(
        store.history("F").unwrap()[0].operation.phase,
        WorkflowDebugPhase::Stopping
    );
    let applied = store.apply_after_stop("F", "k", 1, |_, _| Ok(())).unwrap();
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
        .apply_after_stop("F", "second", 1, |_, _| Ok(()))
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
        {"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"},{"name":"reordered","valueType":"array[string]"}]}},
        {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","items"],"collectSelector":["agent","output"]}}},
        {"id":"agent","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"cli","modelId":"m"},"prompt":"{{#iter.item#}}"}}},
        {"id":"out","data":{"kind":"output","outputs":[{"name":"result","variableSelector":["iter","output"]}]}}],
        "edges":[{"source":"start","target":"iter"},{"source":"iter","target":"agent"},{"source":"iter","target":"out"}]});
    store
        .pool
        .with_connection(|db| {
            bind_graph(db, graph)?;
            let mut payload = ora_application::WorkflowRunPayload::default();
            for (key, value) in [
                ("start.items", json!(["a", "b", "c", "d", "e"])),
                ("start.reordered", json!(["e", "c"])),
            ] {
                payload.variable_pool.declare(key, "array[string]", "start");
                payload.variable_pool.set(key, "start", value).unwrap();
            }
            db.execute(
                "UPDATE workflow_runs SET payload=?1 WHERE id='other'",
                [serde_json::to_string(&payload)?],
            )?;
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
        .with_connection(|db| {
            insert_attempt(db, "out-before", "out", "output", /*index*/ None)
        })
        .unwrap();
    let mut request = edit("new-{{#iter.item#}}", 0);
    request.run_id = "other".into();
    request.selection = WorkflowDebugSelection::NodeRunId("f2".into());
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
        .apply_after_stop("other", "confirm", 1, |tx, plan| {
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
    let container_request=serde_json::from_value(json!({"runId":"other","nodeRunId":"it1","baseRevision":1,
        "patch":[{"op":"replace","path":"/iterationConfig/iteratorSelector","value":["start","reordered"]}]})).unwrap();
    let container = store.preview(&container_request, impact).unwrap();
    assert_eq!(
        container.member_configurations.unwrap()["agent"]["agentConfig"]["prompt"],
        "{{#iter.item#}}"
    );
    assert_eq!(attempt("f1").revision, Some(0));
    store
        .pool
        .with_connection(|db| {
            db.execute(
                "UPDATE workflow_node_runs SET is_deleted=1 WHERE id='f1'",
                [],
            )?;
            insert_attempt(db, "prefix-again", "agent", "agent", Some(1))
        })
        .unwrap();
    let prefix = attempt("prefix-again");
    assert_eq!(
        (
            prefix.revision,
            prefix.configuration.unwrap()["agentConfig"]["prompt"].clone()
        ),
        (Some(1), json!("{{#iter.item#}}"))
    );
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
    let unsettled = [ora_domain::WorkflowNodeRunId::new("t3")];
    store
        .pool
        .with_connection(|db| insert_attempt(db, "other-agent", "agent", "agent", None))
        .unwrap();
    assert!(
        !store
            .record_stop_failure(
                "F",
                "key",
                "wrong run",
                &[ora_domain::WorkflowNodeRunId::new("other-agent")]
            )
            .unwrap()
    );
    assert!(
        store
            .record_stop_failure("F", "key", "still running", &unsettled)
            .unwrap()
    );
    assert!(matches!(
        store.preview(&edit("next", 0), impact),
        Err(DatabaseError::WorkflowDebug(
            DebugRejection::OperationActive
        ))
    ));
    // A failed stop keeps its first reason; a later report cannot rewrite the receipt.
    assert!(
        !store
            .record_stop_failure("F", "key", "late report", &[])
            .unwrap()
    );
    let failed = WorkflowDebugPhase::StopFailed {
        reason: "still running".into(),
        unsettled_node_run_ids: vec!["t3".into()],
    };
    let DebugClaim::Existing(replayed) = store.claim(&commit).unwrap() else {
        panic!("replayed stop");
    };
    assert_eq!(replayed.operation.phase, failed);
    assert_eq!(
        store
            .history("F")
            .unwrap()
            .iter()
            .map(|entry| entry.operation.phase.clone())
            .collect::<Vec<_>>(),
        vec![failed]
    );
    store.release_failed_stop("F", "key").unwrap();
    assert!(store.preview(&edit("next", 0), impact).is_ok());
}

/// The fence outranks every other entry: a new confirmation learns about it before its own read
/// set, and recovery, restart and snapshot switching leave the run untouched until it lifts.
#[test]
fn active_operation_fences_new_confirmations_and_recovery_entries() {
    let (_temp, _path, store) = fixture();
    store
        .record_agent_dispatch(
            "F",
            "t3",
            &json!({"variablePool":WorkflowVariablePool::default()}),
        )
        .unwrap();
    let held = store.preview(&edit("one", 0), impact).unwrap();
    let stale = store.preview(&edit("two", 0), impact).unwrap();
    let commit = |preview: &WorkflowDebugPreview, key: &str| CommitWorkflowDebugRequest {
        run_id: "F".into(),
        preview_id: preview.preview_id.clone(),
        request_id: key.into(),
    };
    assert!(matches!(
        store.claim(&commit(&held, "key")),
        Ok(DebugClaim::Claimed(_))
    ));
    // The target settled by itself, which is enough to make the second preview stale.
    store
        .pool
        .with_connection(|db| {
            Ok(db
                .execute("UPDATE workflow_node_runs SET status=2 WHERE id='t3'", [])
                .map(|_| ())?)
        })
        .unwrap();
    fn active<T>(result: Result<T, DatabaseError>) -> bool {
        matches!(
            result,
            Err(DatabaseError::WorkflowDebug(
                DebugRejection::OperationActive
            ))
        )
    }
    assert!(active(store.claim(&commit(&stale, "other"))));
    assert!(active(store.ensure_debug_operation_inactive("F")));
    let repository = crate::SqliteWorkflowRunEngineRepository::new(store.pool.clone());
    let run = ora_domain::WorkflowRunId::new("F");
    fn fenced<T: std::fmt::Debug>(result: Result<T, ora_application::RepositoryError>) -> bool {
        matches!(
            std::error::Error::source(&result.unwrap_err())
                .and_then(|source| source.downcast_ref::<DatabaseError>()),
            Some(DatabaseError::WorkflowDebug(
                DebugRejection::OperationActive
            ))
        )
    }
    let snapshot = || ora_domain::WorkflowSnapshotId::new("snap");
    assert!(fenced(repository.restart_run(&run, 2)));
    assert!(fenced(repository.resume_from_failure(
        &run,
        &["agent".into()],
        2
    )));
    assert!(fenced(repository.switch_run_snapshot(
        &run,
        &snapshot(),
        "{}",
        2
    )));
    // A failed stop keeps the same fence until the coordinator releases it.
    store
        .record_stop_failure("F", "key", "still running", &[])
        .unwrap();
    assert!(fenced(repository.restart_run(&run, 3)));
    store.release_failed_stop("F", "key").unwrap();
    assert!(store.ensure_debug_operation_inactive("F").is_ok());
    assert!(store.ensure_debug_operation_inactive("other").is_ok());
    // Without the fence the second preview fails on its own facts again.
    assert!(matches!(
        store.claim(&commit(&stale, "other")),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
    assert_eq!(
        repository.restart_run(&run, 4).unwrap(),
        ora_application::RestartWorkflowRunResult::NotRestartable
    );
    assert_eq!(
        repository
            .resume_from_failure(&run, &["agent".into()], 4)
            .unwrap(),
        ora_application::ResumeWorkflowRunResult::NotResumable
    );
    assert!(
        !repository
            .switch_run_snapshot(&run, &snapshot(), "{}", 4)
            .unwrap()
    );
}

/// Reads the status, error and finish time of run `F`.
fn run_row(store: &WorkflowDebugStore) -> (i64, Option<String>, Option<i64>) {
    store
        .pool
        .with_connection(|db| {
            Ok(db.query_row(
                "SELECT run_status,error,finished_at FROM workflow_runs WHERE id='F'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?)
        })
        .unwrap()
}

/// A run accepted in a terminal state keeps its own outcome while the stop is in flight, and
/// returns to running only in the commit that applies the revision.
#[test]
fn terminal_run_returns_to_running_with_the_applied_revision() {
    let (_temp, path, store) = fixture();
    store
        .record_agent_dispatch(
            "F",
            "t3",
            &json!({"variablePool":WorkflowVariablePool::default()}),
        )
        .unwrap();
    // The target failed and took the run with it.
    store
        .pool
        .with_connection(|db| {
            db.execute("UPDATE workflow_node_runs SET status=3 WHERE id='t3'", [])?;
            db.execute(
                "UPDATE workflow_runs SET run_status=3,error='E',finished_at=5 WHERE id='F'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let preview = store.preview(&edit("fixed", 0), impact).unwrap();
    let commit = CommitWorkflowDebugRequest {
        run_id: "F".into(),
        preview_id: preview.preview_id,
        request_id: "k".into(),
    };
    assert!(matches!(
        store.claim(&commit).unwrap(),
        DebugClaim::Claimed(_)
    ));
    assert_eq!(run_row(&store), (3, Some("E".into()), Some(5)));
    let applied = store
        .apply_after_stop("F", "k", 7, |tx, _| {
            tx.execute("UPDATE workflow_node_runs SET is_deleted=1 WHERE id='t3'", [])?;
            tx.execute(
                "INSERT INTO workflow_node_runs(id,run_id,scope_id,node_id,node_type,status,created_at,updated_at)
                SELECT 't3-new','F',scope_id,'agent','agent',0,7,7
                FROM workflow_run_root_scopes WHERE run_id='F'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(applied.phase, WorkflowDebugPhase::Applied { revision: 1 });
    assert_eq!(run_row(&store), (1, None, None));
    let reopened = WorkflowDebugStore::new(RepositoryPool::new(&path).unwrap());
    let history = reopened.history("F").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        (
            history[0].prior_run_status,
            history[0].prior_error.as_deref()
        ),
        (WorkflowRunStatus::Failed, Some("E"))
    );
    let attempt = |id: &str| {
        reopened
            .attempt(&GetWorkflowDebugAttemptRequest {
                run_id: "F".into(),
                node_run_id: id.into(),
            })
            .unwrap()
    };
    // The replacement starts under the new revision. The failed attempt stays readable under
    // the one it actually used.
    let replacement = attempt("t3-new");
    assert_eq!(
        (replacement.revision, replacement.status),
        (Some(1), WorkflowNodeStatus::Pending)
    );
    let retired = attempt("t3");
    assert_eq!(
        (
            retired.revision,
            retired.superseded,
            retired.retired_by_operation_id,
            retired.configuration.unwrap()["agentConfig"]["prompt"].clone(),
        ),
        (
            Some(0),
            true,
            Some(history[0].operation.operation_id.clone()),
            json!("old"),
        )
    );
}

/// A run that reached a new outcome while the stop was in flight keeps that outcome: only the
/// revision applies, and failure recovery reads it later.
#[test]
fn revision_applies_without_reopening_a_run_that_moved_on() {
    let (_temp, _path, store) = fixture();
    store
        .record_agent_dispatch(
            "F",
            "t3",
            &json!({"variablePool":WorkflowVariablePool::default()}),
        )
        .unwrap();
    let preview = store.preview(&edit("one", 0), impact).unwrap();
    let commit = CommitWorkflowDebugRequest {
        run_id: "F".into(),
        preview_id: preview.preview_id,
        request_id: "k".into(),
    };
    assert!(matches!(
        store.claim(&commit).unwrap(),
        DebugClaim::Claimed(_)
    ));
    let history = store.history("F").unwrap();
    assert_eq!(
        (history[0].prior_run_status, history[0].prior_error.clone()),
        (WorkflowRunStatus::Running, None)
    );
    // An independent branch failed while the coordinator was stopping the target.
    store
        .pool
        .with_connection(|db| {
            Ok(db
                .execute(
                    "UPDATE workflow_runs SET run_status=3,error='sibling' WHERE id='F'",
                    [],
                )
                .map(|_| ())?)
        })
        .unwrap();
    assert_eq!(
        store
            .apply_after_stop("F", "k", 9, |_, _| Ok(()))
            .unwrap()
            .phase,
        WorkflowDebugPhase::Applied { revision: 1 }
    );
    assert_eq!(run_row(&store), (3, Some("sibling".into()), None));
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

/// Only a successful snapshot switch starts a new configuration baseline. Earlier revisions stay
/// readable as history and are never replayed, also after switching back; their previews go stale.
#[test]
fn snapshot_switch_starts_a_new_configuration_baseline() {
    let (_temp, _path, store) = fixture();
    let edit_on = |node_run: &str, value: &str, revision: u32| PreviewWorkflowDebugRequest {
        run_id: "other".into(),
        selection: WorkflowDebugSelection::NodeRunId(node_run.into()),
        base_revision: revision,
        patch: vec![WorkflowDebugPatch::Replace {
            path: "/agentConfig/prompt".into(),
            value: json!(value),
        }],
    };
    let attempt = |id: &str| {
        store
            .attempt(&GetWorkflowDebugAttemptRequest {
                run_id: "other".into(),
                node_run_id: id.into(),
            })
            .unwrap()
    };
    let prompt = |id: &str| attempt(id).configuration.unwrap()["agentConfig"]["prompt"].clone();
    // Replaces the live attempt of the only node and records the input the preview needs.
    let admit = |id: &str| {
        store
            .pool
            .with_connection(|db| {
                db.execute(
                    "UPDATE workflow_node_runs SET is_deleted=1 WHERE run_id='other'",
                    [],
                )?;
                insert_attempt(db, id, "agent", "agent", None)
            })
            .unwrap();
        store
            .record_agent_dispatch(
                "other",
                id,
                &json!({"variablePool":WorkflowVariablePool::default()}),
            )
            .unwrap();
    };
    let confirm = |request: &PreviewWorkflowDebugRequest, key: &str| {
        let preview = store.preview(request, impact).unwrap();
        let claim = CommitWorkflowDebugRequest {
            run_id: "other".into(),
            preview_id: preview.preview_id,
            request_id: key.into(),
        };
        assert!(matches!(
            store.claim(&claim).unwrap(),
            DebugClaim::Claimed(_)
        ));
        store
            .apply_after_stop("other", key, 2, |_, _| Ok(()))
            .unwrap();
    };
    admit("a1");
    confirm(&edit_on("a1", "one", 0), "first");
    admit("a2");
    assert_eq!(
        (attempt("a2").revision, prompt("a2")),
        (Some(1), json!("one"))
    );
    let before = store.preview(&edit_on("a2", "two", 1), impact).unwrap();
    store
        .pool
        .with_connection(|db| {
            db.execute(
                "INSERT INTO workflow_snapshots(id,workflow_id,version,graph,created_at)
                 SELECT 'fresh','wf','2',json_set(graph,'$.nodes[1].data.agentConfig.prompt','fresh'),1
                 FROM workflow_snapshots WHERE id='snap'",
                [],
            )?;
            db.execute(
                "UPDATE workflow_runs SET run_status=3,error='E' WHERE id='other'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let repository = crate::SqliteWorkflowRunEngineRepository::new(store.pool.clone());
    let run = ora_domain::WorkflowRunId::new("other");
    let payload = serde_json::to_string(&ora_application::WorkflowRunPayload::default()).unwrap();
    let switch = |snapshot: &str, now: i64| {
        repository.switch_run_snapshot(
            &run,
            &ora_domain::WorkflowSnapshotId::new(snapshot),
            &payload,
            now,
        )
    };
    assert!(switch("fresh", 3).unwrap());
    // The switch raised the run revision: receipts from before it cannot be confirmed or reused.
    assert!(matches!(
        store.claim(&CommitWorkflowDebugRequest {
            run_id: "other".into(),
            preview_id: before.preview_id,
            request_id: "late".into(),
        }),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
    assert!(matches!(
        store.preview(&edit_on("a2", "three", 1), impact),
        Err(DatabaseError::WorkflowDebug(DebugRejection::StalePreview))
    ));
    admit("a3");
    assert_eq!(
        (attempt("a3").revision, prompt("a3")),
        (Some(2), json!("fresh"))
    );
    // The edit made before the switch is history only: a new attempt does not inherit it.
    assert_eq!(prompt("a2"), json!("one"));
    confirm(&edit_on("a3", "three", 2), "second");
    admit("a4");
    assert_eq!(prompt("a4"), json!("three"));
    // Returning to the first snapshot does not bring its edits back either.
    store
        .pool
        .with_connection(|db| {
            Ok(db
                .execute(
                    "UPDATE workflow_runs SET run_status=3,error='E' WHERE id='other'",
                    [],
                )
                .map(|_| ())?)
        })
        .unwrap();
    assert!(switch("snap", 4).unwrap());
    // Attempts recorded before the new baseline are no longer targets.
    assert!(matches!(
        store.preview(&edit_on("a4", "four", 4), impact),
        Err(DatabaseError::WorkflowDebug(DebugRejection::MissingFact(reason)))
            if reason.contains("a4") && reason.contains("baseline 4")
    ));
    admit("a5");
    assert_eq!(
        (attempt("a5").revision, prompt("a5")),
        (Some(4), json!("old"))
    );
    // A refused switch leaves the baseline and the bound snapshot alone.
    store
        .pool
        .with_connection(|db| {
            Ok(db
                .execute("UPDATE workflow_runs SET run_status=1 WHERE id='other'", [])
                .map(|_| ())?)
        })
        .unwrap();
    assert!(!switch("fresh", 5).unwrap());
    assert_eq!(
        store
            .preview(&edit_on("a5", "five", 4), impact)
            .unwrap()
            .base_revision,
        4
    );
    assert_eq!(
        store
            .history("other")
            .unwrap()
            .iter()
            .map(|entry| entry.operation.phase.clone())
            .collect::<Vec<_>>(),
        vec![
            WorkflowDebugPhase::Applied { revision: 1 },
            WorkflowDebugPhase::Applied { revision: 3 },
        ]
    );
}

/// A whole-run restart takes the current revision as its baseline: the new root keeps top-level
/// edits, while a rebuilt container starts from its entrance configuration and overrides bound to
/// the previous container execution stay history.
#[test]
fn restart_keeps_top_level_revisions_but_not_container_overrides() {
    let (_temp, _path, store) = fixture();
    let graph = json!({"nodes":[
        {"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"}]}},
        {"id":"top","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"cli","modelId":"m"},"prompt":"top-old"}}},
        {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","items"],"collectSelector":["agent","output"]}}},
        {"id":"agent","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"cli","modelId":"m"},"prompt":"{{#iter.item#}}"}}}],
        "edges":[{"source":"start","target":"top"},{"source":"start","target":"iter"},{"source":"iter","target":"agent"}]});
    store
        .pool
        .with_connection(|db| {
            bind_graph(db, graph)?;
            let mut payload = ora_application::WorkflowRunPayload::default();
            payload
                .variable_pool
                .declare("start.items", "array[string]", "start");
            payload
                .variable_pool
                .set("start.items", "start", json!(["a", "b", "c"]))
                .unwrap();
            db.execute(
                "UPDATE workflow_runs SET payload=?1 WHERE id='other'",
                [serde_json::to_string(&payload)?],
            )?;
            insert_attempt(db, "t1", "top", "agent", None)?;
            insert_attempt(db, "it1", "iter", "iteration", None)?;
            insert_attempt(db, "f2", "agent", "agent", Some(2))
        })
        .unwrap();
    let mut item = WorkflowVariablePool::default();
    item.declare("iter.item", "string", "iter");
    item.set("iter.item", "iter", json!("c")).unwrap();
    store
        .record_agent_dispatch(
            "other",
            "t1",
            &json!({"variablePool":WorkflowVariablePool::default()}),
        )
        .unwrap();
    store
        .record_agent_dispatch("other", "f2", &json!({"variablePool":item}))
        .unwrap();
    let confirm = |node_run: &str, value: Value, revision: u32, key: &str| {
        let request = PreviewWorkflowDebugRequest {
            run_id: "other".into(),
            selection: WorkflowDebugSelection::NodeRunId(node_run.into()),
            base_revision: revision,
            patch: vec![WorkflowDebugPatch::Replace {
                path: "/agentConfig/prompt".into(),
                value,
            }],
        };
        let preview = store.preview(&request, impact).unwrap();
        let claim = CommitWorkflowDebugRequest {
            run_id: "other".into(),
            preview_id: preview.preview_id,
            request_id: key.into(),
        };
        assert!(matches!(
            store.claim(&claim).unwrap(),
            DebugClaim::Claimed(_)
        ));
        store
            .apply_after_stop("other", key, 2, |_, _| Ok(()))
            .unwrap();
    };
    confirm("t1", json!("top-new"), 0, "top");
    confirm("f2", json!("new-{{#iter.item#}}"), 1, "member");
    let prompt = |id: &str| {
        store
            .attempt(&GetWorkflowDebugAttemptRequest {
                run_id: "other".into(),
                node_run_id: id.into(),
            })
            .unwrap()
            .configuration
            .unwrap()["agentConfig"]["prompt"]
            .clone()
    };
    // The edited attempt keeps what it ran with; the override reaches the later items of the
    // same container execution.
    store
        .pool
        .with_connection(|db| insert_attempt(db, "f3", "agent", "agent", Some(3)))
        .unwrap();
    assert_eq!(
        (prompt("f2"), prompt("f3")),
        (json!("{{#iter.item#}}"), json!("new-{{#iter.item#}}"))
    );
    let repository = crate::SqliteWorkflowRunEngineRepository::new(store.pool.clone());
    store
        .pool
        .with_connection(|db| {
            Ok(db
                .execute(
                    "UPDATE workflow_runs SET run_status=3,error='E' WHERE id='other'",
                    [],
                )
                .map(|_| ())?)
        })
        .unwrap();
    assert_eq!(
        repository
            .restart_run(&ora_domain::WorkflowRunId::new("other"), 5)
            .unwrap(),
        ora_application::RestartWorkflowRunResult::Restarted
    );
    store
        .pool
        .with_connection(|db| {
            insert_attempt(db, "t2", "top", "agent", None)?;
            insert_attempt(db, "it2", "iter", "iteration", None)?;
            insert_attempt(db, "n2", "agent", "agent", Some(2))
        })
        .unwrap();
    assert_eq!(
        (prompt("t2"), prompt("n2"), prompt("f3")),
        (
            json!("top-new"),
            json!("{{#iter.item#}}"),
            json!("new-{{#iter.item#}}")
        )
    );
}

/// The run page only offers variables that are declared as arrays at the root. A path into a
/// nested array is reported on its own field and leaves nothing to confirm.
#[test]
fn iteration_source_must_name_a_root_array_variable() {
    let (_temp, _path, store) = fixture();
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
            insert_attempt(db, "it", "iter", "iteration", None)
        })
        .unwrap();
    let request = |value: Value| PreviewWorkflowDebugRequest {
        run_id: "other".into(),
        selection: WorkflowDebugSelection::NodeRunId("it".into()),
        base_revision: 0,
        patch: vec![WorkflowDebugPatch::Replace {
            path: "/iterationConfig/iteratorSelector".into(),
            value,
        }],
    };
    let field_error = |value: Value| {
        matches!(
            store.preview(&request(value), impact),
            Err(DatabaseError::WorkflowDebug(DebugRejection::InvalidPatch { path, .. }))
                if path == "/iterationConfig/iteratorSelector"
        )
    };
    assert!(field_error(json!(["start", "items", 0])));
    assert!(field_error(json!(["start"])));
    assert!(field_error(json!("start.items")));
    assert!(store.history("other").unwrap().is_empty());
    // A variable declared as an array at the root stays editable.
    store
        .preview(&request(json!(["start", "items"])), impact)
        .unwrap();
}
