use super::*;
use ora_application::WorkflowRunPayload;
use pretty_assertions::assert_eq;

/// A container edit stops its active member's session, not the sessionless container.
/// An unrelated failure delivered while that stop is pending wins over redispatch.
#[test]
fn foreach_debug_preserves_a_run_that_failed_while_the_member_was_stopping() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let config = json!({"executor":{"agentCli":"c","modelId":"m"},"prompt":"run","retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0}});
        let graph=json!({"nodes":[{"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"}]}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","items"],"collectSelector":["member","output"],"errorStrategy":"continue"}}},
            {"id":"member","parentId":"iter","data":{"kind":"agent","agentConfig":config}},
            {"id":"sibling","data":{"kind":"agent","agentConfig":config}}],"edges":[{"source":"start","target":"iter"},{"source":"start","target":"sibling"},{"source":"iter","target":"member"}]}).to_string();
        let run = seeded_pending_run(&temp, &pool, &graph);
        let repo = SqliteWorkflowRunEngineRepository::new(pool.clone());
        repo.update_run_input(
            &run,
            None,
            [("items".into(), json!(["one", "two"]))].into(),
            35,
        )
        .unwrap();
        let exec = RecordingExecutor::default();
        let engine = Arc::new(WorkflowRunEngine::new(
            repo.clone(),
            exec.clone(),
            UuidWorkflowNodeRunIdGenerator::new(),
            SystemClock,
        ));
        let gate = KeyedResourceLocks::new();
        let barrier = Arc::new(super::lifecycle::StopBarrier::default());
        let transitions = Arc::new(WorkflowRunTransitions::new(
            pool.clone(),
            Arc::new(Invalidations::default()),
            gate.clone(),
        ));
        let coordinator = WorkflowDebugCoordinator::new(
            pool.clone(),
            gate,
            barrier.clone(),
            engine.clone(),
            transitions,
            SystemClock,
        );
        engine.start(&run).unwrap();
        let rows = node_runs(&pool, &run);
        let id = |name| &rows.iter().find(|r| r.node_id == name).unwrap().id;
        repo.bind_node_run_session(id("member"), &SessionId::new("session-1"), 50)
            .unwrap();
        let preview=coordinator.preview(serde_json::from_value(json!({"runId":run.as_ref(),"nodeRunId":id("iter").as_ref(),"baseRevision":0,"patch":[{"op":"replace","path":"/iterationConfig/errorStrategy","value":"fail"}]})).unwrap()).unwrap();
        assert_eq!(preview.impact.stop, vec![id("member").to_string()]);
        let request = CommitWorkflowDebugRequest {
            run_id: run.to_string(),
            preview_id: preview.preview_id,
            request_id: "finish-while-stopping".into(),
        };
        let load = || repo.find_execution_context(&run).unwrap().unwrap().run;
        let (operation, failure) = tokio::join!(coordinator.commit(request), async {
            barrier.entered.notified().await;
            engine
                .fail_node(
                    &run,
                    id("sibling"),
                    ora_application::NodeFailure::new(
                        ora_application::NodeFailureKind::Session,
                        "sibling failed",
                    ),
                )
                .unwrap();
            let failure = load();
            barrier.release.notify_one();
            failure
        });
        assert_eq!(
            operation.unwrap().phase,
            WorkflowDebugPhase::Applied { revision: 1 }
        );
        let after = load();
        assert_eq!(after.status, WorkflowRunStatus::Failed);
        assert_eq!(
            (after.error, after.finished_at, after.payload),
            (failure.error, failure.finished_at, failure.payload)
        );
        assert_eq!(node_runs(&pool, &run).len(), rows.len());
        assert_eq!(dispatched(&exec, "member"), 1);
    });
}

/// Re-evaluate a previously skipped branch from its selected item: the old Condition row
/// remains historical while the new per-item decisions determine the collected outputs.
#[test]
fn foreach_debug_condition_uses_the_selected_item_and_preserves_the_previous_route() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let graph=json!({"nodes":[{"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"}]}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","items"],"collectSelector":["fix","output"],"errorStrategy":"continue"}}},
            {"id":"gate","parentId":"iter","data":{"kind":"condition","cases":[{"id":"match","logic":"and","conditions":[{"variableSelector":["iter","item"],"operator":"equals","value":"a"}]}]}},
            {"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{"prompt":"fix","executor":{"agentCli":"c","modelId":"m"}}}},
            {"id":"sibling","data":{"kind":"agent","agentConfig":{"prompt":"hold","executor":{"agentCli":"c","modelId":"m"}}}}],
            "edges":[{"source":"start","target":"sibling"},{"source":"start","target":"iter"},{"source":"iter","target":"gate"},{"source":"gate","sourceHandle":"match","target":"fix"}]}).to_string();
        let run = seeded_pending_run(&temp, &pool, &graph);
        let repo = SqliteWorkflowRunEngineRepository::new(pool.clone());
        repo.update_run_input(
            &run,
            None,
            [("items".into(), json!(["a", "b", "c"]))].into(),
            35,
        )
        .unwrap();
        let exec = RecordingExecutor::default();
        let engine = Arc::new(WorkflowRunEngine::new(
            repo.clone(),
            exec.clone(),
            UuidWorkflowNodeRunIdGenerator::new(),
            SystemClock,
        ));
        let gate = KeyedResourceLocks::new();
        let transitions = Arc::new(WorkflowRunTransitions::new(
            pool.clone(),
            Arc::new(Invalidations::default()),
            gate.clone(),
        ));
        let coordinator = WorkflowDebugCoordinator::new(
            pool.clone(),
            gate,
            Arc::new(NoSessions),
            engine.clone(),
            transitions,
            SystemClock,
        );
        engine.start(&run).unwrap();
        let running = || {
            node_runs(&pool, &run)
                .into_iter()
                .find(|r| r.node_id == "fix" && r.status == WorkflowNodeStatus::Running)
                .unwrap()
        };
        engine
            .complete_node(
                &run,
                &running().id,
                Some("first".to_owned()),
                None,
                None,
                vec![],
            )
            .unwrap();
        let target = node_runs(&pool, &run)
            .into_iter()
            .find(|r| r.node_id == "gate" && r.iteration == Some(1))
            .unwrap();
        let sibling = node_runs(&pool, &run)
            .into_iter()
            .find(|row| row.node_id == "sibling")
            .unwrap();
        engine
            .complete_node(&run, &sibling.id, Some("done".into()), None, None, vec![])
            .unwrap();
        assert_eq!(
            repo.find_execution_context(&run)
                .unwrap()
                .unwrap()
                .run
                .status,
            WorkflowRunStatus::Succeeded
        );
        let preview=coordinator.preview(serde_json::from_value(json!({"runId":run.as_ref(),"nodeRunId":target.id.as_ref(),"baseRevision":0,"patch":[{"op":"replace","path":"/cases/0/conditions/0/value","value":"b"}]})).unwrap()).unwrap();
        assert_eq!(preview.impact.predicted_branch.as_deref(), Some("match"));
        coordinator
            .commit(CommitWorkflowDebugRequest {
                run_id: run.to_string(),
                preview_id: preview.preview_id,
                request_id: "route".into(),
            })
            .await
            .unwrap();
        let f = running();
        assert_eq!(
            repo.find_execution_context(&run)
                .unwrap()
                .unwrap()
                .run
                .status,
            WorkflowRunStatus::Running
        );
        assert_eq!(f.iteration, Some(1));
        engine
            .complete_node(&run, &f.id, Some("second".to_owned()), None, None, vec![])
            .unwrap();
        let ctx = repo.find_execution_context(&run).unwrap().unwrap();
        assert_eq!(ctx.run.status, WorkflowRunStatus::Succeeded);
        let payload: WorkflowRunPayload =
            serde_json::from_str(ctx.run.payload.as_deref().unwrap()).unwrap();
        assert_eq!(
            payload.variable_pool.values["iter.output"],
            json!(["first", "second"])
        );
        assert_eq!(payload.variable_pool.values["iter.failed_count"], json!(1));
        assert_eq!(dispatched(&exec, "fix"), 2);
        let historical = WorkflowDebugStore::new(pool.clone())
            .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                run_id: run.to_string(),
                node_run_id: target.id.to_string(),
            })
            .unwrap();
        assert!(historical.superseded);
        assert_eq!(historical.selected_branch.as_deref(), Some("else"));
        // Re-edit the same effective item. The second revision replaces the suffix;
        // the newly skipped item must not retain its previous collected output.
        let current = node_runs(&pool, &run)
            .into_iter()
            .find(|r| r.node_id == "gate" && r.iteration == Some(1))
            .unwrap();
        let next=coordinator.preview(serde_json::from_value(json!({"runId":run.as_ref(),"nodeRunId":current.id.as_ref(),"baseRevision":1,"patch":[{"op":"replace","path":"/cases/0/conditions/0/value","value":"c"}]})).unwrap()).unwrap();
        coordinator
            .commit(CommitWorkflowDebugRequest {
                run_id: run.to_string(),
                preview_id: next.preview_id,
                request_id: "route-again".into(),
            })
            .await
            .unwrap();
        let f = running();
        assert_eq!(f.iteration, Some(2));
        engine
            .complete_node(&run, &f.id, Some("third".into()), None, None, vec![])
            .unwrap();
        let ctx = repo.find_execution_context(&run).unwrap().unwrap();
        let payload: WorkflowRunPayload =
            serde_json::from_str(ctx.run.payload.as_deref().unwrap()).unwrap();
        assert_eq!(
            payload.variable_pool.values["iter.output"],
            json!(["first", "third"])
        );
        assert_eq!(
            payload.variable_pool.values["iter.entries"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(payload.variable_pool.values["iter.failed_count"], json!(1));
        assert_eq!(dispatched(&exec, "fix"), 3);
    });
}

/// The fifth item has overwritten the pool when the user edits the third. Drive the public
/// coordinator and actual scheduler, recording calls rather than inferring them from the ledger.
#[test]
fn foreach_debug_replays_from_the_selected_member_without_repeating_its_predecessor() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let agent = |id: &str| json!({"id":id,"parentId":"iter","data":{"kind":"agent","agentConfig":{"prompt":"old","executor":{"agentCli":"c","modelId":"m"},"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0}}}});
        let graph = json!({"nodes":[{"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"},{"name":"short","valueType":"array[string]"}]}},
            {"id":"sibling","data":{"kind":"agent","agentConfig":{"prompt":"keep","executor":{"agentCli":"c","modelId":"m"}}}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","items"],"collectSelector":["fix","output"],"errorStrategy":"continue"}}},agent("prep"),agent("fix"),
            {"id":"out","data":{"kind":"output","outputs":[{"name":"result","variableSelector":["iter","output"]}]}}],
            "edges":[{"source":"start","target":"sibling"},{"source":"start","target":"iter"},{"source":"iter","target":"prep"},{"source":"prep","target":"fix"},{"source":"iter","target":"out"}]}).to_string();
        let run = seeded_pending_run(&temp, &pool, &graph);
        let repo = SqliteWorkflowRunEngineRepository::new(pool.clone());
        repo.update_run_input(
            &run,
            None,
            [
                ("items".into(), json!(["a", "b", "c", "d", "e"])),
                ("short".into(), json!(["e", "c"])),
            ]
            .into(),
            35,
        )
        .unwrap();
        let exec = RecordingExecutor::default();
        let engine = Arc::new(WorkflowRunEngine::new(
            repo.clone(),
            exec.clone(),
            UuidWorkflowNodeRunIdGenerator::new(),
            SystemClock,
        ));
        let gate = KeyedResourceLocks::new();
        let transitions = Arc::new(WorkflowRunTransitions::new(
            pool.clone(),
            Arc::new(Invalidations::default()),
            gate.clone(),
        ));
        let coordinator = WorkflowDebugCoordinator::new(
            pool.clone(),
            gate,
            Arc::new(NoSessions),
            engine.clone(),
            transitions,
            SystemClock,
        );
        let store = WorkflowDebugStore::new(pool.clone());
        engine.start(&run).unwrap();
        let mut selected = None;
        let active = || {
            node_runs(&pool, &run)
                .into_iter()
                .find(|r| r.iteration.is_some() && r.status == WorkflowNodeStatus::Running)
                .unwrap()
        };
        let state = || {
            serde_json::from_str::<WorkflowRunPayload>(
                repo.find_execution_context(&run)
                    .unwrap()
                    .unwrap()
                    .run
                    .payload
                    .as_deref()
                    .unwrap(),
            )
            .unwrap()
        };
        for i in 0..5 {
            let p = active();
            assert_eq!((&*p.node_id, p.iteration), ("prep", Some(i)));
            engine
                .complete_node(&run, &p.id, Some(format!("prep-{i}")), None, None, vec![])
                .unwrap();
            let f = active();
            if i == 2 {
                let edit = || {
                    serde_json::from_value(json!({"runId":run.as_ref(),"nodeRunId":f.id.as_ref(),"baseRevision":0,"patch":[{"op":"replace","path":"/agentConfig/prompt","value":"new"}]})).unwrap()
                };
                let err = coordinator.preview(edit()).unwrap_err();
                assert!(error_chain(&err).contains("missing_input_evidence"));
                assert!(
                    store
                        .record_agent_dispatch(
                            run.as_ref(),
                            f.id.as_ref(),
                            &json!({"variablePool":state().variable_pool})
                        )
                        .unwrap()
                );
                selected = Some(f.id.clone());
                engine
                    .fail_node(
                        &run,
                        &f.id,
                        ora_application::NodeFailure::new(
                            ora_application::NodeFailureKind::Session,
                            "old failure",
                        ),
                    )
                    .unwrap();
            } else if i != 4 {
                engine
                    .complete_node(&run, &f.id, Some(format!("old-{i}")), None, None, vec![])
                    .unwrap();
            }
        }
        repo.bind_node_run_session(&active().id, &SessionId::new("session-1"), 50)
            .unwrap();
        let preview = coordinator.preview(serde_json::from_value(json!({"runId":run.as_ref(),"nodeRunId":selected.unwrap().as_ref(),"baseRevision":0,"patch":[{"op":"replace","path":"/agentConfig/prompt","value":"new {{#prep.output#}}"}]})).unwrap()).unwrap();
        assert_eq!(preview.impact.stop, vec![active().id.to_string()]);
        coordinator
            .commit(CommitWorkflowDebugRequest {
                run_id: run.to_string(),
                preview_id: preview.preview_id,
                request_id: "third".into(),
            })
            .await
            .unwrap();
        for (i, item) in [(2, "c"), (3, "d"), (4, "e")] {
            if i != 2 {
                let p = active();
                assert_eq!((&*p.node_id, p.iteration), ("prep", Some(i)));
                engine
                    .complete_node(&run, &p.id, Some(format!("prep-{i}")), None, None, vec![])
                    .unwrap();
            }
            let f = active();
            assert_eq!((&*f.node_id, f.iteration), ("fix", Some(i)));
            let values = state().variable_pool.values;
            assert_eq!(
                (
                    &values["iter.item"],
                    &values["iter.index"],
                    &values["prep.output"]
                ),
                (&json!(item), &json!(i), &json!(format!("prep-{i}")))
            );
            let revised = store
                .revised_graph(run.as_ref(), f.id.as_ref())
                .unwrap()
                .unwrap();
            assert_eq!(
                revised
                    .node("fix")
                    .unwrap()
                    .agent_config
                    .as_ref()
                    .unwrap()
                    .prompt,
                "new {{#prep.output#}}"
            );
            engine
                .complete_node(&run, &f.id, Some(format!("new-{i}")), None, None, vec![])
                .unwrap();
        }
        assert_eq!(
            state().variable_pool.values["iter.output"],
            json!(["old-0", "old-1", "new-2", "new-3", "new-4"])
        );
        let observed: Vec<_> = exec
            .records()
            .into_iter()
            .filter(|dispatch| dispatch.node_id != "sibling")
            .map(|dispatch| {
                let state: WorkflowRunPayload =
                    serde_json::from_str(dispatch.payload.as_deref().unwrap()).unwrap();
                (
                    dispatch.node_id,
                    state.variable_pool.values["iter.index"].as_u64().unwrap(),
                )
            })
            .collect();
        let expected: Vec<_> = (0..5)
            .flat_map(|i| [("prep".to_owned(), i), ("fix".to_owned(), i)])
            .chain([("fix".to_owned(), 2)])
            .chain((3..5).flat_map(|i| [("prep".to_owned(), i), ("fix".to_owned(), i)]))
            .collect();
        assert_eq!(observed, expected);
        assert_eq!(state().variable_pool.values["iter.failed_count"], json!(0));
        assert_eq!(
            state().variable_pool.values["iter.entries"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        let owner = node_runs(&pool, &run)
            .into_iter()
            .find(|row| row.node_id == "iter")
            .unwrap();
        let restart = coordinator.preview(serde_json::from_value(json!({"runId":run.as_ref(),"nodeRunId":owner.id.as_ref(),"baseRevision":1,"patch":[{"op":"replace","path":"/iterationConfig/iteratorSelector","value":["start","short"]}]})).unwrap()).unwrap();
        assert_eq!(
            restart.member_configurations.as_ref().unwrap()["fix"]["agentConfig"]["prompt"],
            json!("old")
        );
        coordinator
            .commit(CommitWorkflowDebugRequest {
                run_id: run.to_string(),
                preview_id: restart.preview_id,
                request_id: "shorten".into(),
            })
            .await
            .unwrap();
        for (i, item) in ["e", "c"].into_iter().enumerate() {
            let p = active();
            assert_eq!((&*p.node_id, p.iteration), ("prep", Some(i as u32)));
            assert_eq!(state().variable_pool.values["iter.item"], json!(item));
            engine
                .complete_node(
                    &run,
                    &p.id,
                    Some(format!("prep-{item}")),
                    None,
                    None,
                    vec![],
                )
                .unwrap();
            let f = active();
            let graph = store
                .revised_graph(run.as_ref(), f.id.as_ref())
                .unwrap()
                .unwrap();
            assert_eq!(
                graph
                    .node("fix")
                    .unwrap()
                    .agent_config
                    .as_ref()
                    .unwrap()
                    .prompt,
                "old"
            );
            engine
                .complete_node(&run, &f.id, Some(item.to_owned()), None, None, vec![])
                .unwrap();
        }
        assert_eq!(
            state().variable_pool.values["iter.output"],
            json!(["e", "c"])
        );
        let current_rows = node_runs(&pool, &run);
        let container = current_rows
            .iter()
            .find(|row| row.node_id == "iter")
            .unwrap();
        let preview = coordinator
            .preview(serde_json::from_value(json!({
                "runId": run.as_ref(),
                "nodeRunId": container.id.as_ref(),
                "baseRevision": 2,
                "patch": [{"op":"replace","path":"/iterationConfig/collectSelector","value":["prep","output"]}]
            })).unwrap())
            .unwrap();
        coordinator
            .commit(CommitWorkflowDebugRequest {
                run_id: run.to_string(),
                preview_id: preview.preview_id,
                request_id: "collect-prep".into(),
            })
            .await
            .unwrap();
        for item in ["e", "c"] {
            for member in ["prep", "fix"] {
                let attempt = active();
                assert_eq!(attempt.node_id, member);
                engine
                    .complete_node(
                        &run,
                        &attempt.id,
                        Some(format!("{member}-{item}")),
                        None,
                        None,
                        vec![],
                    )
                    .unwrap();
            }
        }
        let sibling = current_rows
            .iter()
            .find(|row| row.node_id == "sibling")
            .unwrap();
        engine
            .complete_node(&run, &sibling.id, Some("done".into()), None, None, vec![])
            .unwrap();
        let finished = repo.find_execution_context(&run).unwrap().unwrap().run;
        assert_eq!(finished.status, WorkflowRunStatus::Succeeded);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(finished.output.as_deref().unwrap()).unwrap(),
            json!({"result":["prep-e","prep-c"]})
        );
        assert_eq!(dispatched(&exec, "sibling"), 1);
    });
}
