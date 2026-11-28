use super::*;
use pretty_assertions::assert_eq;

/// Two independent members enter each item together. Only B owns the collection.
fn parallel_batch(collect: &str) -> Fixture {
    let agent = |id: &str| {
        json!({"id":id,"parentId":"iter","data":{"kind":"agent",
            "agentConfig":{"prompt":"old","executor":{"agentCli":"c","modelId":"m"},
                "retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0}}}})
    };
    let mut b = agent("b");
    if collect == "structured_output" {
        b["data"]["agentConfig"]["outputContract"] = json!({"type":"structured",
                "textExposure":"includeFinalText","schema":{"type":"object"}});
    }
    let graph = json!({"nodes":[
            {"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"}]}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{"iteratorSelector":["start","items"],"collectSelector":["b",collect],"errorStrategy":"continue"}}},
            agent("a"),b],
            "edges":[{"source":"start","target":"iter"},{"source":"iter","target":"a"},{"source":"iter","target":"b"}]});
    Fixture::with_inputs(
        Arc::new(NoSessions),
        &graph.to_string(),
        [("items".into(), json!(["first", "second"]))].into(),
    )
}

/// A retained producer can finish after its peer's dispatch. The next item overwrites
/// the same variable before the user returns to edit the first item.
#[test]
fn foreach_reuses_the_parallel_producers_committed_item_output() {
    run_test(async {
        let f = parallel_batch("output");
        let selected = active(&f, "a");
        evidence(&f, &selected, &payload(&f).variable_pool);
        complete(&f, "a", "old");
        let retained = active(&f, "b");
        complete(&f, "b", "first B");
        complete(&f, "a", "old");
        complete(&f, "b", "second B");

        let preview = f.preview(selected.id.as_ref(), 0);
        assert!(preview.impact.keep.contains(&retained.id.to_string()));
        f.coordinator
            .commit(f.commit_request(&preview, "parallel"))
            .await
            .unwrap();
        complete(&f, "a", "edited");
        assert_eq!(
            payload(&f).iteration_ledger["iter"][&0].to_envelope()["output"],
            json!("first B")
        );
        complete(&f, "a", "edited again");
        complete(&f, "b", "new second B");
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(
            payload(&f).variable_pool.values["iter.output"],
            json!(["first B", "new second B"])
        );
        assert_eq!(
            (dispatched(&f.executor, "a"), dispatched(&f.executor, "b")),
            (4, 3)
        );
        let old = f
            .store
            .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                run_id: f.run_id.to_string(),
                node_run_id: selected.id.to_string(),
            })
            .unwrap();
        assert!(old.superseded);
    });
}

/// The original structured value is provable only if the first item settled
/// successfully. B's final text is different, and the live pool belongs to item two.
#[test]
fn foreach_preview_keeps_a_proven_object_or_recomputes_its_producer() {
    for settled in [true, false] {
        run_test(async {
            let f = parallel_batch("structured_output");
            let selected = active(&f, "a");
            let producer = active(&f, "b");
            evidence(&f, &selected, &payload(&f).variable_pool);
            let publish = |value| {
                f.engine
                    .complete_node(
                        &f.run_id,
                        &active(&f, "b").id,
                        Some("final text".into()),
                        Some(json!({"value":value})),
                        None,
                        vec![],
                    )
                    .unwrap()
            };
            publish(1);
            if settled {
                complete(&f, "a", "old");
            } else {
                f.engine
                    .fail_node(
                        &f.run_id,
                        &selected.id,
                        NodeFailure::new(NodeFailureKind::Session, "first failed"),
                    )
                    .unwrap();
            }
            complete(&f, "a", "second");
            publish(2);
            let preview = f.preview(selected.id.as_ref(), 0);
            assert_eq!(
                preview.impact.keep.contains(&producer.id.to_string()),
                settled
            );
            assert_eq!(
                preview.impact.invalidate.contains(&producer.id.to_string()),
                !settled
            );
            if !settled {
                assert!(preview.impact.schedule.contains(&"b".to_owned()));
            }
            f.coordinator
                .commit(f.commit_request(&preview, "recompute"))
                .await
                .unwrap();
            complete(&f, "a", "edited");
            if !settled {
                publish(3);
            }
            complete(&f, "a", "edited again");
            publish(4);
            let first = if settled { 1 } else { 3 };
            assert_eq!(
                payload(&f).variable_pool.values["iter.output"],
                json!([{"value":first},{"value":4}])
            );
            assert_eq!(
                (dispatched(&f.executor, "a"), dispatched(&f.executor, "b")),
                (4, if settled { 3 } else { 4 })
            );
            let old = f
                .store
                .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                    run_id: f.run_id.to_string(),
                    node_run_id: producer.id.to_string(),
                })
                .unwrap();
            assert_eq!(old.superseded, !settled);
            assert_eq!(old.output.as_deref(), Some("final text"));
        });
    }
}
