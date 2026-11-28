//! Revisions spanning multiple physical containers in one workflow.
use super::*;
use pretty_assertions::assert_eq;

/// Only the foreach with a parked replay retains its owner and settled prefix on recovery.
#[test]
fn ordinary_resume_distinguishes_two_foreach_owners_in_one_run() {
    run_test(async {
        let barrier = Arc::new(super::super::super::lifecycle::StopBarrier::default());
        let mut graph = batch_definition(true);
        let mut other = graph["nodes"][1].clone();
        other["id"] = json!("other");
        other["data"]["iterationConfig"]["collectSelector"] = json!(["worker", "output"]);
        let mut worker = graph["nodes"][2].clone();
        worker["id"] = json!("worker");
        worker["parentId"] = json!("other");
        graph["nodes"]
            .as_array_mut()
            .unwrap()
            .extend([other, worker]);
        graph["edges"].as_array_mut().unwrap().extend([
            json!({"source":"start","target":"other"}),
            json!({"source":"other","target":"worker"}),
        ]);
        let f = Fixture::with_inputs(
            barrier.clone(),
            &graph.to_string(),
            [("items".into(), json!(["a", "b"]))].into(),
        );
        let old = active(&f, "other").id;
        let parked = active(&f, "iter").id;
        let prefix = active(&f, "member").id;
        f.engine
            .fail_node(
                &f.run_id,
                &active(&f, "worker").id,
                NodeFailure::new(NodeFailureKind::Session, "absorbed"),
            )
            .unwrap();
        complete(&f, "worker", "second");
        assert_eq!(
            payload(&f).variable_pool.values["other.failed_count"],
            json!(1)
        );
        complete(&f, "member", "first");
        let selected = active(&f, "member");
        evidence(&f, &selected, &payload(&f).variable_pool);
        complete(&f, "member", "old");
        bind(&f, &active(&f, "consumer"));
        let preview = f.preview(selected.id.as_ref(), 0);
        let (operation, ()) = tokio::join!(
            f.coordinator.commit(f.commit_request(&preview, "park")),
            async {
                barrier.entered.notified().await;
                f.engine
                    .fail_node(
                        &f.run_id,
                        &active(&f, "sibling").id,
                        NodeFailure::new(NodeFailureKind::Session, "independent"),
                    )
                    .unwrap();
                barrier.release.notify_one();
            }
        );
        assert_eq!(
            operation.unwrap().phase,
            WorkflowDebugPhase::Applied { revision: 1 }
        );
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        assert_ne!(active(&f, "other").id, old);
        assert_eq!(active(&f, "worker").iteration, Some(0));
        assert_eq!(active(&f, "iter").id, parked);
        assert!(node_runs(&f.pool, &f.run_id).iter().any(|r| r.id == prefix));
        let revised = active(&f, "member");
        assert_eq!(revised.iteration, Some(1));
        assert_eq!(
            f.store
                .revised_graph(f.run_id.as_ref(), revised.id.as_ref())
                .unwrap()
                .unwrap()
                .node("member")
                .unwrap()
                .agent_config
                .as_ref()
                .unwrap()
                .prompt,
            "new"
        );
        complete(&f, "worker", "retried");
        complete(&f, "worker", "second");
        complete(&f, "member", "revised");
        assert_eq!(
            payload(&f).variable_pool.values["other.output"],
            json!(["retried", "second"])
        );
        assert_eq!(
            payload(&f).variable_pool.values["iter.output"],
            json!(["first", "revised"])
        );
    });
}

/// A foreach edit retires all current members of the downstream Loop even if the run ends during stop.
#[test]
fn foreach_edit_retires_downstream_loop_round_on_apply_and_terminal_race() {
    run_test(async {
        for fail_during_stop in [false, true] {
            let barrier = Arc::new(super::super::super::lifecycle::StopBarrier::default());
            let mut graph: serde_json::Value =
                serde_json::from_str(crate::workflow::run::loop_resume_tests::LOOP_GRAPH).unwrap();
            let batch = batch_definition(true);
            graph["nodes"][0] = batch["nodes"][0].clone();
            let mut iter = batch["nodes"][1].clone();
            iter["data"]["iterationConfig"]["collectSelector"] = json!(["fix", "output"]);
            let mut prep = batch["nodes"][2].clone();
            prep["id"] = json!("prep");
            let mut fix = prep.clone();
            fix["id"] = json!("fix");
            graph["nodes"].as_array_mut().unwrap().extend([
                iter,
                prep,
                fix,
                batch["nodes"][5].clone(),
            ]);
            graph["edges"][0]["source"] = json!("iter");
            graph["edges"].as_array_mut().unwrap().extend([
                json!({"source":"start","target":"iter"}),
                json!({"source":"iter","target":"prep"}),
                json!({"source":"prep","target":"fix"}),
                json!({"source":"start","target":"sibling"}),
            ]);
            let f = Fixture::with_inputs(
                barrier.clone(),
                &graph.to_string(),
                [("items".into(), json!(["a", "b"]))].into(),
            );
            complete(&f, "prep", "p0");
            complete(&f, "fix", "f0");
            complete(&f, "prep", "p1");
            let selected = active(&f, "fix");
            evidence(&f, &selected, &payload(&f).variable_pool);
            complete(&f, "fix", "f1");
            let writer = active(&f, "writer");
            bind(&f, &writer);
            let preview = f.preview(selected.id.as_ref(), 0);
            assert!(preview.impact.invalidate.contains(&writer.id.to_string()));
            assert_eq!(preview.impact.stop, vec![writer.id.to_string()]);
            let (op, ()) = tokio::join!(
                f.coordinator.commit(f.commit_request(&preview, "revise")),
                async {
                    barrier.entered.notified().await;
                    if fail_during_stop {
                        f.engine
                            .fail_node(
                                &f.run_id,
                                &active(&f, "sibling").id,
                                NodeFailure::new(NodeFailureKind::Session, "independent"),
                            )
                            .unwrap();
                    }
                    barrier.release.notify_one();
                }
            );
            assert_eq!(
                op.unwrap().phase,
                WorkflowDebugPhase::Applied { revision: 1 }
            );
            let detail = SqliteWorkflowRunRepository::new(f.pool.clone())
                .get_run_detail(&f.run_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                detail
                    .scopes
                    .iter()
                    .find(|s| s.id == writer.scope_id)
                    .unwrap()
                    .status,
                ora_domain::WorkflowScopeStatus::Cancelled
            );
            assert!(!detail.nodes.iter().any(|r| r.id == writer.id));
            if fail_during_stop {
                assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
            } else {
                assert_eq!(payload(&f).variable_pool.values["iter.item"], json!("b"));
                assert_eq!(payload(&f).variable_pool.values["prep.output"], json!("p1"));
                complete(&f, "fix", "revised");
                assert_ne!(active(&f, "writer").scope_id, writer.scope_id);
                assert_eq!(
                    node_runs(&f.pool, &f.run_id)
                        .iter()
                        .filter(|r| r.node_id == "writer")
                        .count(),
                    1
                );
            }
        }
    });
}
