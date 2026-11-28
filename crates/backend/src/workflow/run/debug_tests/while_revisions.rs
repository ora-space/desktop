//! End-to-end while revision boundaries exercised against SQLite and the real coordinator.
use super::super::loop_resume_tests::LOOP_GRAPH;
use super::*;
use pretty_assertions::assert_eq;

impl Fixture {
    /// Return the currently dispatched writer together with its observed prompt and input.
    fn live_writer(&self) -> super::super::test_fixture::DispatchRecord {
        self.executor
            .records()
            .into_iter()
            .rev()
            .find(|r| r.node_id == "writer")
            .unwrap()
    }

    /// Record the executor handoff before driving the next production transition.
    fn observe_writer(&self) -> WorkflowNodeRunId {
        let writer = self.live_writer();
        let id = WorkflowNodeRunId::new(writer.node_run_id);
        self.store
            .record_agent_dispatch(
                self.run_id.as_ref(),
                id.as_ref(),
                &json!({"variablePool":writer.variable_pool}),
            )
            .unwrap();
        id
    }

    /// Drive a Loop writer to its normal completion (and any newly ready descendants).
    fn answer(&self, value: &str) {
        self.engine
            .complete_node(
                &self.run_id,
                &WorkflowNodeRunId::new(self.live_writer().node_run_id),
                Some(value.into()),
                None,
                None,
                vec![],
            )
            .unwrap();
    }
}

/// A late stop acknowledgement must not resurrect a newly failed run.
#[test]
fn sibling_failure_during_member_stop_keeps_the_run_failed_without_redispatch() {
    run_test(async {
        let mut graph: serde_json::Value = serde_json::from_str(LOOP_GRAPH).unwrap();
        graph["nodes"].as_array_mut().unwrap().push(json!({
            "id":"outside", "data":{"kind":"agent", "agentConfig":{
                "retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},
                "executor":{"agentCli":"c","modelId":"m"}, "prompt":"other"
            }}
        }));
        graph["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":"start","target":"outside"}));
        let barrier = Arc::new(super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &graph.to_string());
        let selected = f.observe_writer();
        f.bind_agent(&selected);
        let preview = f.preview(selected.as_ref(), 0);
        let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
        // The parked round reopen is recovery bookkeeping on the owner, not part of the terminal
        // projection; `recovery_races` covers what recovery does with it.
        let projections = || {
            db.query_row(
            "SELECT json_array(r.run_status, r.state, r.payload, n.status,
                    json_remove(COALESCE(n.payload, '{}'), '$.debugWhileReplays'), s.status, s.state)
             FROM workflow_runs r JOIN workflow_execution_scopes s ON s.run_id = r.id
             JOIN workflow_node_runs n ON n.id = s.parent_loop_node_run_id WHERE s.id = ?1",
            [&preview.target.scope_id], |r| r.get::<_, String>(0),
        ).unwrap()
        };
        let (op, terminal_state) = tokio::join!(
            f.coordinator.commit(f.commit_request(&preview, "race")),
            async {
                barrier.entered.notified().await;
                f.engine
                    .fail_node(
                        &f.run_id,
                        &f.row("outside").id,
                        ora_application::NodeFailure::new(
                            ora_application::NodeFailureKind::InvalidRunPayload,
                            "outside failed",
                        ),
                    )
                    .unwrap();
                assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
                let terminal_state = projections();
                barrier.release.notify_one();
                terminal_state
            }
        );
        let op = op.unwrap();
        assert_eq!(projections(), terminal_state);
        assert_eq!(op.phase, WorkflowDebugPhase::Applied { revision: 1 });
        assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
        assert_eq!(dispatched(&f.executor, "writer"), 1);
        assert!(out_rows(&f.pool, &f.run_id).is_empty());
        for id in &preview.impact.invalidate {
            let retired = f
                .store
                .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                    run_id: f.run_id.to_string(),
                    node_run_id: id.clone(),
                })
                .unwrap();
            assert!(retired.superseded);
            assert_eq!(
                retired.retired_by_operation_id.as_ref(),
                Some(&op.operation_id)
            );
        }
        let archived: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM workflow_debug_round_history WHERE scope_id=?1 AND operation_id=?2)",
            (&preview.target.scope_id, &op.operation_id), |r| r.get(0),
        ).unwrap();
        assert!(archived);
    });
}

/// Reopen from the persisted outcome, not from a manually manufactured terminal row.
#[test]
fn resume_real_terminal_runs_from_the_selected_member() {
    run_test(async {
        for terminal in [
            WorkflowRunStatus::Succeeded,
            WorkflowRunStatus::Failed,
            WorkflowRunStatus::Cancelled,
        ] {
            let f = Fixture::with_graph(Arc::new(NoSessions), LOOP_GRAPH);
            f.engine
                .record_node_checkpoint(&f.row("loop").id, "snapshot-1", Some("pre-loop"), None)
                .unwrap();
            f.answer("S1");
            f.answer("S2");
            let target = f.observe_writer();
            match terminal {
                WorkflowRunStatus::Succeeded => f.answer("done"),
                WorkflowRunStatus::Failed => f.answer("limit"),
                WorkflowRunStatus::Cancelled => {
                    f.engine.cancel(&f.run_id).unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(f.run_status(), terminal);
            let p = f.preview(target.as_ref(), 0);
            assert_eq!(p.target.round, Some(3));
            let op = f
                .coordinator
                .commit(f.commit_request(&p, "reopen"))
                .await
                .unwrap();
            assert_eq!(op.phase, WorkflowDebugPhase::Applied { revision: 1 });
            assert_eq!(f.run_status(), WorkflowRunStatus::Running);
            let payload: serde_json::Value =
                serde_json::from_str(f.row("loop").payload.as_deref().unwrap()).unwrap();
            assert_eq!(payload["checkpoint"], json!("pre-loop"));
            let replacement = f.live_writer();
            assert_eq!(
                (
                    replacement.prompt.as_deref(),
                    replacement.variable_pool.values["loop.draft"].as_str()
                ),
                (Some("new"), Some("S2"))
            );
            f.answer("done");
            assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
            assert_eq!(dispatched(&f.executor, "writer"), 4);
            assert_eq!(f.row("out").output.as_deref(), Some(r#"{"draft":"done"}"#));
        }
    });
}

/// Even a valid prompt cannot authorize restoring a member with another round's input.
#[test]
fn rejects_mismatched_carried_evidence_before_issuing_a_preview() {
    run_test(async {
        let f = Fixture::with_graph(Arc::new(NoSessions), LOOP_GRAPH);
        f.answer("second");
        let mut writer = f.live_writer();
        writer
            .variable_pool
            .values
            .insert("loop.draft".into(), json!("seed"));
        assert!(
            f.store
                .record_agent_dispatch(
                    f.run_id.as_ref(),
                    &writer.node_run_id,
                    &json!({"variablePool":writer.variable_pool})
                )
                .unwrap()
        );
        let request = json!({
            "runId": f.run_id.as_ref(), "nodeRunId": writer.node_run_id, "baseRevision": 0,
            "patch": [{"op":"replace", "path":"/agentConfig/prompt", "value":"new"}],
        });
        let error = f
            .coordinator
            .preview(serde_json::from_value(request).unwrap())
            .unwrap_err();
        assert!(error_chain(&error).contains("missing_input_evidence"));
        assert_eq!(dispatched(&f.executor, "writer"), 2);
        assert!(f.store.history(f.run_id.as_ref()).unwrap().is_empty());
    });
}

/// Only the container edit crosses the entrance. The previous member override belongs
/// to its original owner, including when that owner's row is later read from history.
#[test]
fn editing_only_feedback_or_until_rebuilds_the_entrance_and_discards_old_overrides() {
    run_test(async {
        for edit_feedback in [false, true] {
            let f = Fixture::with_graph(Arc::new(NoSessions), LOOP_GRAPH);
            let old_parent = f.row("loop");
            let start = f.row("start");
            f.answer("first");
            f.answer("OLD");
            let member = f.observe_writer();
            f.bind_agent(&member);
            let p = f.preview(member.as_ref(), 0);
            f.coordinator
                .commit(f.commit_request(&p, "member"))
                .await
                .unwrap();
            let edited = f.live_writer();
            assert_eq!(edited.prompt.as_deref(), Some("new"));
            f.bind_agent(&WorkflowNodeRunId::new(&edited.node_run_id));
            let patch = if edit_feedback {
                json!({"op":"replace","path":"/loopConfig/variables/0/feedback","value":["loop","draft"]})
            } else {
                json!({"op":"replace", "path":"/loopConfig/until", "value":{
                    "logic":"and", "conditions":[{
                        "variableSelector":["writer","output"], "operator":"equals", "value":"FIRST"
                    }]
                }})
            };
            let request = json!({
                "runId": f.run_id.as_ref(), "nodeRunId": old_parent.id.as_ref(), "baseRevision": 1,
                "patch": [patch],
            });
            let p = f
                .coordinator
                .preview(serde_json::from_value(request).unwrap())
                .unwrap();
            assert_eq!(
                p.member_configurations.as_ref().unwrap()["writer"]["agentConfig"]["prompt"],
                json!("revise")
            );
            let op = f
                .coordinator
                .commit(f.commit_request(&p, "container"))
                .await
                .unwrap();
            assert_eq!(op.phase, WorkflowDebugPhase::Applied { revision: 2 });
            assert_ne!(f.row("loop").id, old_parent.id);
            let first = f.live_writer();
            assert_eq!(
                (
                    first.prompt.as_deref(),
                    first.variable_pool.values["loop.draft"].as_str()
                ),
                (Some("revise"), Some("seed"))
            );
            f.answer("FIRST");
            if edit_feedback {
                assert_eq!(
                    f.live_writer().variable_pool.values["loop.draft"],
                    json!("seed")
                );
                f.answer("SECOND");
                assert_eq!(f.live_writer().prompt.as_deref(), Some("revise"));
                f.answer("done");
            }
            assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
            assert_eq!(f.row("start").id, start.id);
            assert_eq!(
                f.row("out").output.as_deref(),
                Some(if edit_feedback {
                    r#"{"draft":"done"}"#
                } else {
                    r#"{"draft":"FIRST"}"#
                })
            );
            let old = f
                .store
                .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                    run_id: f.run_id.to_string(),
                    node_run_id: edited.node_run_id,
                })
                .unwrap();
            assert!(old.superseded);
            assert_eq!(
                old.configuration.unwrap()["agentConfig"]["prompt"],
                json!("new")
            );
        }
    });
}

/// A live exit wins over independent edits, while undoing its own path releases its cancelled writers.
#[test]
fn rewinding_an_exit_reopens_only_its_cancelled_siblings() {
    run_test(async {
        let mut graph: serde_json::Value =
            serde_json::from_str(&super::super::loop_exit_tests::graph(true)).unwrap();
        let mut tail = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "slow")
            .unwrap()
            .clone();
        tail["id"] = json!("tail");
        graph["nodes"].as_array_mut().unwrap().push(tail);
        graph["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":"slow","target":"tail"}));
        let f = Fixture::with_graph(Arc::new(NoSessions), &graph.to_string());
        let slow = f.row("slow");
        let handoff = f
            .executor
            .records()
            .into_iter()
            .find(|r| r.node_id == "slow")
            .unwrap();
        f.store
            .record_agent_dispatch(
                f.run_id.as_ref(),
                slow.id.as_ref(),
                &json!({"variablePool": handoff.variable_pool}),
            )
            .unwrap();
        f.answer("done");
        let exit = f.row("exit");
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        let preview = |id: &WorkflowNodeRunId, path: &str| {
            let request = json!({
                "runId": f.run_id.as_ref(), "nodeRunId": id, "baseRevision": 0,
                "patch": [{"op": "replace", "path": path, "value": "continue"}],
            });
            f.coordinator
                .preview(serde_json::from_value(request).unwrap())
        };
        let error = preview(&slow.id, "/agentConfig/prompt").unwrap_err();
        assert!(error_chain(&error).contains("invalid_target"));
        assert!(f.store.history(f.run_id.as_ref()).unwrap().is_empty());
        let check = f.row("check");
        let p = preview(&check.id, "/cases/0/conditions/0/value").unwrap();
        assert!(p.impact.invalidate.contains(&exit.id.to_string()));
        assert!(p.impact.invalidate.contains(&slow.id.to_string()));
        assert!(p.impact.schedule.contains(&"tail".to_string()));
        f.coordinator
            .commit(f.commit_request(&p, "undo-exit"))
            .await
            .unwrap();
        assert_eq!(dispatched(&f.executor, "slow"), 2);
        assert_eq!(dispatched(&f.executor, "again"), 1);
        assert_eq!(dispatched(&f.executor, "writer"), 1);
        assert_ne!(f.row("slow").id, slow.id);
        assert_eq!(f.row("slow").scope_id, slow.scope_id);
        f.engine
            .complete_node(
                &f.run_id,
                &f.row("slow").id,
                Some("ok".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        assert_eq!(dispatched(&f.executor, "tail"), 1);
    });
}

/// A claim with no external writer releases the run gate before apply. A sibling
/// completion in that interval must not publish the old round's feedback or output.
#[test]
fn sibling_completion_does_not_advance_a_fenced_round_with_no_stopped_agent() {
    run_test(async {
        let mut graph: serde_json::Value = serde_json::from_str(LOOP_GRAPH).unwrap();
        let mut slow = graph["nodes"][3].clone();
        slow["id"] = json!("slow");
        graph["nodes"].as_array_mut().unwrap().push(slow);
        graph["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":"entry", "target":"slow"}));
        let f = Fixture::with_graph(Arc::new(NoSessions), &graph.to_string());
        let selected = f.observe_writer();
        f.answer("done");
        let p = f.preview(selected.as_ref(), 0);
        assert!(p.impact.stop.is_empty());
        let request = f.commit_request(&p, "hold-round");
        f.store.claim(&request).unwrap();
        // This callback may acquire the gate after claim and before the coordinator reacquires it.
        f.engine
            .complete_node(
                &f.run_id,
                &f.row("slow").id,
                Some("finished".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        let owner = f.row("loop");
        assert_eq!(
            (owner.status, owner.output),
            (WorkflowNodeStatus::Running, None)
        );
        assert_eq!(f.run_status(), WorkflowRunStatus::Running);
        assert!(out_rows(&f.pool, &f.run_id).is_empty());
        f.store
            .apply_after_stop(&request.run_id, &request.request_id, 100, |tx, p| {
                super::super::debug::strategy::settle(tx, &request.run_id, 100, p)
            })
            .unwrap();
        f.engine.resume(&f.run_id).unwrap();
        assert_eq!(f.live_writer().prompt.as_deref(), Some("new"));
        assert_eq!(dispatched(&f.executor, "slow"), 1);
        f.answer("done");
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
    });
}

/// A selected writer can precede a different Loop whose live writer must stop before redispatch.
#[test]
fn rewinding_a_member_waits_for_a_downstream_loop_writer() {
    run_test(async {
        let mut graph: serde_json::Value = serde_json::from_str(LOOP_GRAPH).unwrap();
        let mut second = graph["nodes"][1].clone();
        second["id"] = json!("next");
        let config = &mut second["data"]["loopConfig"];
        config["variables"][0]["feedback"][0] = json!("nextWriter");
        config["until"]["conditions"][0]["variableSelector"][0] = json!("nextWriter");
        config["outputs"][0]["variableSelector"][0] = json!("nextWriter");
        let mut entry = graph["nodes"][2].clone();
        let mut writer = graph["nodes"][3].clone();
        for (node, id) in [(&mut entry, "nextEntry"), (&mut writer, "nextWriter")] {
            node["id"] = json!(id);
            node["parentId"] = json!("next");
            node["data"]["containerId"] = json!("next");
        }
        graph["nodes"]
            .as_array_mut()
            .unwrap()
            .extend([second, entry, writer]);
        graph["edges"][2]["target"] = json!("next");
        graph["edges"].as_array_mut().unwrap().extend([
            json!({"source":"nextEntry","target":"nextWriter"}),
            json!({"source":"next","target":"out"}),
        ]);
        let barrier = Arc::new(super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &graph.to_string());
        let selected = f.observe_writer();
        f.answer("done");
        let downstream = f.row("nextWriter");
        f.bind_agent(&downstream.id);
        let p = f.preview(selected.as_ref(), 0);
        assert_eq!(p.impact.stop, vec![downstream.id.to_string()]);
        assert!(p.impact.invalidate.contains(&downstream.id.to_string()));
        assert!(p.impact.schedule.contains(&downstream.node_id));
        let (op, ()) = tokio::join!(f.coordinator.commit(f.commit_request(&p, "chain")), async {
            barrier.entered.notified().await;
            assert_eq!(dispatched(&f.executor, "writer"), 1);
            barrier.release.notify_one();
        });
        let op = op.unwrap();
        let history = f
            .store
            .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                run_id: f.run_id.to_string(),
                node_run_id: downstream.id.to_string(),
            })
            .unwrap();
        assert_eq!(history.retired_by_operation_id, Some(op.operation_id));
        assert_eq!(dispatched(&f.executor, "writer"), 2);
        f.answer("done");
        assert_eq!(dispatched(&f.executor, "nextWriter"), 2);
        assert_ne!(f.row("nextWriter").scope_id, downstream.scope_id);
    });
}

/// A valid Loop configuration can name an upstream output absent at the recorded entrance.
#[test]
fn loop_preview_rejects_missing_initial_carried_value() {
    run_test(async {
        let mut graph: serde_json::Value = serde_json::from_str(LOOP_GRAPH).unwrap();
        let mut source = graph["nodes"][3].clone();
        source["id"] = json!("source");
        source.as_object_mut().unwrap().remove("parentId");
        source["data"]
            .as_object_mut()
            .unwrap()
            .remove("containerId");
        graph["edges"][0]["source"] = json!("source");
        graph["nodes"].as_array_mut().unwrap().push(source);
        graph["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":"start","target":"source"}));
        graph["nodes"][1]["data"]["loopConfig"]["variables"][0]["initial"] =
            json!({"kind":"variable","selector":["source","output"]});
        let f = Fixture::with_graph(Arc::new(NoSessions), &graph.to_string());
        f.engine
            .complete_node(
                &f.run_id,
                &f.row("source").id,
                Some("seed".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        let owner = f.row("loop");
        let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
        // Model a partial historical receipt with an intact pool but no value for this selector.
        let changed = db.execute(
            r#"UPDATE workflow_debug_documents SET refs = json_remove(refs, '$."source.output"')
               WHERE node_run_id = ?1 AND kind = 'run' AND json_type(refs, '$."source.output"') IS NOT NULL"#,
            [owner.id.as_ref()],
        ).unwrap();
        assert_eq!(changed, 1);
        let request = json!({
            "runId": f.run_id.as_ref(), "nodeRunId": owner.id, "baseRevision": 0,
            "patch": [{"op":"replace","path":"/loopConfig/maxIterations","value":2}],
        });
        let error = f
            .coordinator
            .preview(serde_json::from_value(request).unwrap())
            .unwrap_err();
        assert!(
            error_chain(&error).contains("missing_input_evidence"),
            "{}",
            error_chain(&error)
        );
        assert!(f.store.history(f.run_id.as_ref()).unwrap().is_empty());
    });
}
