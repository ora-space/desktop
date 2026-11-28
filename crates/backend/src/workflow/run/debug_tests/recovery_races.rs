//! Recovery after a debug edit lost its normal apply path: a crash between acceptance and apply,
//! and an independent failure during the stop followed by ordinary failure recovery (#1616).
use super::super::loop_resume_tests::LOOP_GRAPH;
use super::super::test_fixture::DispatchRecord;
use super::*;
use pretty_assertions::assert_eq;

mod boundaries;
mod withdrawals;

/// LOOP_GRAPH with five rounds and an independent root agent `outside`.
fn five_rounds_with_outside() -> String {
    let mut graph: serde_json::Value =
        serde_json::from_str(&LOOP_GRAPH.replace("\"maxIterations\":3", "\"maxIterations\":5"))
            .unwrap();
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
    graph.to_string()
}

/// The newest dispatch of one node.
fn last_dispatch(executor: &RecordingExecutor, node_id: &str) -> DispatchRecord {
    executor
        .records()
        .into_iter()
        .rev()
        .find(|r| r.node_id == node_id)
        .unwrap()
}

/// One Loop's round history as `(round, operation, awaiting_replay)`, in round/operation order.
fn round_history(f: &Fixture) -> Vec<(u32, String, bool)> {
    let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
    let mut statement = db
        .prepare(
            "SELECT s.round_index, h.operation_id, h.awaiting_replay
             FROM workflow_debug_round_history h JOIN workflow_execution_scopes s ON s.id = h.scope_id
             WHERE s.run_id = ?1 ORDER BY s.round_index, h.operation_id",
        )
        .unwrap();
    statement
        .query_map([f.run_id.as_ref()], |row| {
            Ok((row.get(0)?, row.get::<_, i64>(1)?.to_string(), row.get(2)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

impl Fixture {
    /// Record the newest writer's executor handoff, so it can be previewed and stopped.
    fn observe_live_writer(&self) -> DispatchRecord {
        let writer = last_dispatch(&self.executor, "writer");
        self.store
            .record_agent_dispatch(
                self.run_id.as_ref(),
                &writer.node_run_id,
                &json!({"variablePool":writer.variable_pool}),
            )
            .unwrap();
        writer
    }

    /// Complete the newest writer dispatch with one output.
    fn finish_writer(&self, value: &str) {
        let writer = last_dispatch(&self.executor, "writer");
        self.engine
            .complete_node(
                &self.run_id,
                &WorkflowNodeRunId::new(writer.node_run_id),
                Some(value.into()),
                None,
                None,
                vec![],
            )
            .unwrap();
    }

    /// Preview a prompt replacement on one attempt at a given revision.
    fn prompt_preview(&self, node_run_id: &str, base: u32, prompt: &str) -> WorkflowDebugPreview {
        self.coordinator
            .preview(
                serde_json::from_value(json!({
                    "runId": self.run_id.as_ref(), "nodeRunId": node_run_id, "baseRevision": base,
                    "patch": [{"op":"replace","path":"/agentConfig/prompt","value":prompt}]
                }))
                .unwrap(),
            )
            .unwrap()
    }

    /// Stop round 4's writer for an edit at round 3 while `outside` fails, the reverse race:
    /// the revision applies but the run stays Failed and nothing redispatches.
    async fn edit_round_three_while_outside_fails(
        &self,
        barrier: &super::lifecycle::StopBarrier,
    ) -> (
        DispatchRecord,
        DispatchRecord,
        ora_contracts::WorkflowDebugOperation,
    ) {
        // Keep the sibling's own handoff so a later edit can select it.
        let outside = last_dispatch(&self.executor, "outside");
        self.store
            .record_agent_dispatch(
                self.run_id.as_ref(),
                &outside.node_run_id,
                &json!({"variablePool":outside.variable_pool}),
            )
            .unwrap();
        for value in ["S1", "S2"] {
            self.finish_writer(value);
        }
        let round_three = self.observe_live_writer();
        self.finish_writer("S3");
        let round_four = self.observe_live_writer();
        self.bind_agent(&WorkflowNodeRunId::new(&round_four.node_run_id));
        let preview = self.prompt_preview(&round_three.node_run_id, 0, "new");
        assert_eq!(
            (preview.target.round, preview.impact.stop.clone()),
            (Some(3), vec![round_four.node_run_id.clone()])
        );
        let (operation, ()) = tokio::join!(
            self.coordinator
                .commit(self.commit_request(&preview, "race")),
            async {
                barrier.entered.notified().await;
                self.engine
                    .fail_node(
                        &self.run_id,
                        &self.row("outside").id,
                        ora_application::NodeFailure::new(
                            ora_application::NodeFailureKind::Session,
                            "outside failed",
                        ),
                    )
                    .unwrap();
                barrier.release.notify_one();
            }
        );
        let operation = operation.unwrap();
        assert_eq!(operation.phase, WorkflowDebugPhase::Applied { revision: 1 });
        assert_eq!(self.run_status(), WorkflowRunStatus::Failed);
        assert_eq!(dispatched(&self.executor, "writer"), 4);
        (round_three, round_four, operation)
    }

    /// Scope of one dispatched attempt, read from its persisted row (retired rows included).
    fn scope_of(&self, node_run_id: &str) -> String {
        let db = rusqlite::Connection::open(self._temp.path().join("repository.sqlite3")).unwrap();
        db.query_row(
            "SELECT scope_id FROM workflow_node_runs WHERE id = ?1",
            [node_run_id],
            |row| row.get(0),
        )
        .unwrap()
    }
}

/// D1/D5.5: an edit accepted on a Succeeded run, interrupted before its apply, must not be turned
/// into an `interrupted_by_restart` failure by the boot sweep. Boot applies it, reopens the round,
/// dispatches the edited member, and the run settles from the new result.
#[test]
fn crash_after_accepting_an_edit_on_a_succeeded_run_applies_and_dispatches_it_at_boot() {
    run_test(async {
        // The crash lands after acceptance, or after the apply commit but before its dispatch.
        for applied_before_crash in [false, true] {
            let f = Fixture::with_graph(Arc::new(NoSessions), LOOP_GRAPH);
            let selected = f.observe_live_writer();
            f.finish_writer("done");
            assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
            let preview = f.prompt_preview(&selected.node_run_id, 0, "new");
            assert!(preview.impact.stop.is_empty());
            let request = f.commit_request(&preview, "crash");
            assert!(matches!(
                f.store.claim(&request).unwrap(),
                ora_db::workflow_debug::DebugClaim::Claimed(_)
            ));
            if applied_before_crash {
                f.store
                    .apply_after_stop(f.run_id.as_ref(), "crash", 60, |tx, p| {
                        super::super::debug::strategy::settle(tx, f.run_id.as_ref(), 60, p)
                    })
                    .unwrap();
                assert_eq!(dispatched(&f.executor, "writer"), 1);
            }

            // The accepting process is gone. A new engine, executor and gate run the production boot.
            let executor = RecordingExecutor::default();
            let engine = Arc::new(WorkflowRunEngine::new(
                SqliteWorkflowRunEngineRepository::new(f.pool.clone()),
                executor.clone(),
                UuidWorkflowNodeRunIdGenerator::new(),
                SystemClock,
            ));
            super::super::recovery::run_workflow_run_boot_sweep(
                &f.pool,
                &engine,
                &KeyedResourceLocks::new(),
                SystemClock,
            );

            assert_eq!(
                f.store.history(f.run_id.as_ref()).unwrap()[0]
                    .operation
                    .phase,
                WorkflowDebugPhase::Applied { revision: 1 }
            );
            assert_eq!(f.run_status(), WorkflowRunStatus::Running);
            let owner = f.row("loop");
            assert_eq!(
                (owner.status, owner.error),
                (WorkflowNodeStatus::Running, None)
            );
            let replacement = last_dispatch(&executor, "writer");
            assert_eq!(
                (
                    executor.records().len(),
                    replacement.prompt.as_deref(),
                    replacement.variable_pool.values["loop.draft"].as_str(),
                    f.scope_of(&replacement.node_run_id),
                ),
                (
                    1,
                    Some("new"),
                    Some("seed"),
                    f.scope_of(&selected.node_run_id)
                )
            );
            engine
                .complete_node(
                    &f.run_id,
                    &WorkflowNodeRunId::new(&replacement.node_run_id),
                    Some("done".into()),
                    None,
                    None,
                    vec![],
                )
                .unwrap();
            assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
            assert_eq!(f.row("out").output.as_deref(), Some(r#"{"draft":"done"}"#));
        }
    });
}

/// D4 / invariant 3: after the reverse race, ordinary failure recovery replays the edited round
/// with the new revision instead of continuing the next round from the retired attempt's output.
/// No slot keeps a stale replay marker, so a follow-up edit attributes round 4 to itself.
#[test]
fn ordinary_recovery_after_the_reverse_race_replays_the_edited_round() {
    run_test(async {
        let barrier = Arc::new(super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &five_rounds_with_outside());
        let (round_three, round_four, first) =
            f.edit_round_three_while_outside_fails(&barrier).await;
        // The terminal run keeps its slots: nothing awaits replay until recovery reopens them.
        let op1 = first.operation_id.clone();
        assert_eq!(
            round_history(&f),
            vec![(3, op1.clone(), false), (4, op1.clone(), false)]
        );

        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        let replay = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                replay.prompt.as_deref(),
                replay.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&replay.node_run_id),
            ),
            (
                Some("new"),
                Some("S2"),
                f.scope_of(&round_three.node_run_id)
            )
        );
        assert_eq!(
            round_history(&f),
            vec![(3, op1.clone(), false), (4, op1.clone(), true)]
        );
        f.finish_writer("T3");
        let next = f.observe_live_writer();
        assert_eq!(
            (
                next.prompt.as_deref(),
                next.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&next.node_run_id),
            ),
            (Some("new"), Some("T3"), f.scope_of(&round_four.node_run_id))
        );
        assert_eq!(
            round_history(&f),
            vec![(3, op1.clone(), false), (4, op1.clone(), false)]
        );

        // A follow-up edit on the replayed round 4 owns that slot's retirement.
        f.bind_agent(&WorkflowNodeRunId::new(&next.node_run_id));
        let follow = f.prompt_preview(&next.node_run_id, 1, "newer");
        assert_eq!(follow.impact.stop, vec![next.node_run_id.clone()]);
        let (second, ()) = tokio::join!(
            f.coordinator.commit(f.commit_request(&follow, "follow")),
            async {
                barrier.entered.notified().await;
                barrier.release.notify_one();
            }
        );
        let second = second.unwrap();
        assert_eq!(second.phase, WorkflowDebugPhase::Applied { revision: 2 });
        let op2 = second.operation_id.clone();
        assert_eq!(
            round_history(&f),
            vec![
                (3, op1.clone(), false),
                (4, op1.clone(), false),
                (4, op2.clone(), false)
            ]
        );
        let retired = f
            .store
            .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                run_id: f.run_id.to_string(),
                node_run_id: next.node_run_id.clone(),
            })
            .unwrap();
        assert_eq!(retired.retired_by_operation_id, Some(op2));
        let revised = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                revised.prompt.as_deref(),
                revised.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&revised.node_run_id),
            ),
            (
                Some("newer"),
                Some("T3"),
                f.scope_of(&round_four.node_run_id)
            )
        );
        f.finish_writer("done");
        f.engine
            .complete_node(
                &f.run_id,
                &f.row("outside").id,
                Some("recovered".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(f.row("out").output.as_deref(), Some(r#"{"draft":"done"}"#));
        // Rounds 1-2 ran once, round 3 twice, round 4 three times; no fifth round.
        assert_eq!(dispatched(&f.executor, "writer"), 7);
    });
}

/// The same reverse race recovered by debugging the failed sibling instead: reopening the run for
/// that edit must also replay the parked round, never continue from the retired output.
#[test]
fn debugging_the_failed_sibling_after_the_reverse_race_replays_the_edited_round() {
    run_test(async {
        let barrier = Arc::new(super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &five_rounds_with_outside());
        let (round_three, _, first) = f.edit_round_three_while_outside_fails(&barrier).await;
        let op1 = first.operation_id;
        let outside = f.row("outside");
        let sibling = f.prompt_preview(outside.id.as_ref(), 1, "fixed");
        assert!(sibling.impact.schedule.contains(&"writer".to_string()));
        let op = f
            .coordinator
            .commit(f.commit_request(&sibling, "sibling"))
            .await
            .unwrap();
        assert_eq!(op.phase, WorkflowDebugPhase::Applied { revision: 2 });
        assert_eq!(f.run_status(), WorkflowRunStatus::Running);
        let replay = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                replay.prompt.as_deref(),
                replay.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&replay.node_run_id),
            ),
            (
                Some("new"),
                Some("S2"),
                f.scope_of(&round_three.node_run_id)
            )
        );
        f.finish_writer("done");
        f.engine
            .complete_node(
                &f.run_id,
                &f.row("outside").id,
                Some("recovered".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(f.row("out").output.as_deref(), Some(r#"{"draft":"done"}"#));
        // Round 4 stays archived for this Loop execution: its replay was never needed.
        assert_eq!(
            round_history(&f),
            vec![(3, op1.clone(), false), (4, op1, true)]
        );
    });
}

/// A restart between the reverse race and ordinary recovery must not fail the parked Loop as
/// interrupted: recovery afterwards still replays the edited round with the new revision.
#[test]
fn restart_before_ordinary_recovery_keeps_the_parked_round_replay() {
    run_test(async {
        let barrier = Arc::new(super::lifecycle::StopBarrier::default());
        let f = Fixture::with_graph(barrier.clone(), &five_rounds_with_outside());
        let (round_three, _, _) = f.edit_round_three_while_outside_fails(&barrier).await;
        let repository = SqliteWorkflowRunRepository::new(f.pool.clone());
        let failed = repository.find_run(&f.run_id).unwrap().unwrap();
        let loop_row = f.row("loop");
        super::super::recovery::run_workflow_run_boot_sweep(
            &f.pool,
            &f.engine,
            &KeyedResourceLocks::new(),
            SystemClock,
        );
        assert_eq!(f.row("loop"), loop_row);
        let after = repository.find_run(&f.run_id).unwrap().unwrap();
        assert_eq!(
            (after.status, after.error, after.finished_at),
            (failed.status, failed.error, failed.finished_at)
        );
        assert_eq!(dispatched(&f.executor, "writer"), 4);
        assert_eq!(
            f.engine.resume_from_failure(&f.run_id).unwrap(),
            ora_application::ResumeWorkflowRunResult::Resumed
        );
        let replay = last_dispatch(&f.executor, "writer");
        assert_eq!(
            (
                replay.prompt.as_deref(),
                replay.variable_pool.values["loop.draft"].as_str(),
                f.scope_of(&replay.node_run_id),
            ),
            (
                Some("new"),
                Some("S2"),
                f.scope_of(&round_three.node_run_id)
            )
        );
    });
}

/// Out-of-scope work interrupted by the crash still fails the run (ordinary boot cleanup), but it
/// must not take the accepted Loop edit with it: boot finishes the edit after the sweep, so the
/// round reopen is parked and ordinary recovery replays it with the new revision.
#[test]
fn crash_with_interrupted_independent_work_keeps_the_accepted_loop_edit_for_recovery() {
    run_test(async {
        // The crash lands after acceptance, or after the apply commit but before its dispatch.
        for applied_before_crash in [false, true] {
            let graph = five_rounds_with_outside();
            let f = Fixture::with_graph(Arc::new(NoSessions), &graph);
            let selected = f.observe_live_writer();
            f.finish_writer("done");
            assert_eq!(f.run_status(), WorkflowRunStatus::Running);
            let preview = f.prompt_preview(&selected.node_run_id, 0, "new");
            assert!(preview.impact.stop.is_empty());
            f.store.claim(&f.commit_request(&preview, "crash")).unwrap();
            if applied_before_crash {
                f.store
                    .apply_after_stop(f.run_id.as_ref(), "crash", 60, |tx, p| {
                        super::super::debug::strategy::settle(tx, f.run_id.as_ref(), 60, p)
                    })
                    .unwrap();
            }
            super::super::recovery::run_workflow_run_boot_sweep(
                &f.pool,
                &f.engine,
                &KeyedResourceLocks::new(),
                SystemClock,
            );
            assert_eq!(
                f.store.history(f.run_id.as_ref()).unwrap()[0]
                    .operation
                    .phase,
                WorkflowDebugPhase::Applied { revision: 1 }
            );
            assert_eq!(
                (f.run_status(), f.row("outside").status),
                (WorkflowRunStatus::Failed, WorkflowNodeStatus::Failed)
            );
            assert_eq!(dispatched(&f.executor, "writer"), 1);
            assert_eq!(
                f.engine.resume_from_failure(&f.run_id).unwrap(),
                ora_application::ResumeWorkflowRunResult::Resumed
            );
            let replay = last_dispatch(&f.executor, "writer");
            assert_eq!(
                (
                    replay.prompt.as_deref(),
                    replay.variable_pool.values["loop.draft"].as_str(),
                    f.scope_of(&replay.node_run_id),
                ),
                (Some("new"), Some("seed"), f.scope_of(&selected.node_run_id))
            );
        }
    });
}

/// Control for the exemption above (while D6): without an applied edit that reopened the round,
/// a Running Loop still fails at restart, even when its round has no attempt in flight.
#[test]
fn an_ordinary_idle_loop_still_fails_at_restart() {
    run_test(async {
        let f = Fixture::with_graph(Arc::new(NoSessions), LOOP_GRAPH);
        let db = rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
        // The process stopped between a member's settlement and the round's next step.
        db.execute(
            "UPDATE workflow_node_runs SET status = 2 WHERE id = ?1",
            [f.row("writer").id.as_ref()],
        )
        .unwrap();
        super::super::recovery::run_workflow_run_boot_sweep(
            &f.pool,
            &f.engine,
            &KeyedResourceLocks::new(),
            SystemClock,
        );
        let owner = f.row("loop");
        assert_eq!(
            (f.run_status(), owner.status, owner.error.as_deref()),
            (
                WorkflowRunStatus::Failed,
                WorkflowNodeStatus::Failed,
                Some(r#"{"reason":"interrupted_by_restart"}"#)
            )
        );
        assert_eq!(dispatched(&f.executor, "writer"), 1);
    });
}

/// The boot order matters for foreach as well: an applied member edit reopens its Iteration row,
/// which the sweep would fail with the run's interrupted sibling. Finished after the sweep, the
/// edit parks behind the failed run and ordinary recovery replays only the selected item.
#[test]
fn crash_with_interrupted_independent_work_keeps_the_accepted_foreach_edit_for_recovery() {
    run_test(async {
        // The crash lands after acceptance, or after the apply commit reopened the Iteration
        // but before its item was dispatched (M1 of review 1).
        for applied_before_crash in [false, true] {
            let agent = json!({"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},
            "executor":{"agentCli":"c","modelId":"m"},"prompt":"old"});
            let graph = json!({"nodes":[
            {"id":"start","data":{"kind":"start","inputVariables":[{"name":"items","valueType":"array[string]"}]}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{
                "iteratorSelector":["start","items"],"collectSelector":["member","output"],"errorStrategy":"fail"}}},
            {"id":"member","parentId":"iter","data":{"kind":"agent","agentConfig":agent}},
            {"id":"outside","data":{"kind":"agent","agentConfig":agent}}
        ],"edges":[
            {"source":"start","target":"iter"},{"source":"iter","target":"member"},
            {"source":"start","target":"outside"}
        ]})
        .to_string();
            let f = Fixture::with_inputs(
                Arc::new(NoSessions),
                &graph,
                [("items".to_string(), json!(["a", "b"]))].into(),
            );
            let mut second = None;
            for value in ["A", "B"] {
                let member = last_dispatch(&f.executor, "member");
                f.store
                    .record_agent_dispatch(
                        f.run_id.as_ref(),
                        &member.node_run_id,
                        &json!({"variablePool":member.variable_pool}),
                    )
                    .unwrap();
                second = Some(member.node_run_id.clone());
                f.engine
                    .complete_node(
                        &f.run_id,
                        &WorkflowNodeRunId::new(member.node_run_id),
                        Some(value.into()),
                        None,
                        None,
                        vec![],
                    )
                    .unwrap();
            }
            assert_eq!(f.row("iter").status, WorkflowNodeStatus::Succeeded);
            assert_eq!(f.run_status(), WorkflowRunStatus::Running);
            let preview = f.prompt_preview(&second.unwrap(), 0, "new");
            assert_eq!(
                (preview.target.index, preview.impact.stop.clone()),
                (Some(1), vec![])
            );
            f.store.claim(&f.commit_request(&preview, "crash")).unwrap();
            if applied_before_crash {
                f.store
                    .apply_after_stop(f.run_id.as_ref(), "crash", 60, |tx, p| {
                        super::super::debug::strategy::settle(tx, f.run_id.as_ref(), 60, p)
                    })
                    .unwrap();
                assert_eq!(f.row("iter").status, WorkflowNodeStatus::Running);
            }
            super::super::recovery::run_workflow_run_boot_sweep(
                &f.pool,
                &f.engine,
                &KeyedResourceLocks::new(),
                SystemClock,
            );
            assert_eq!(
                (f.run_status(), f.row("outside").status),
                (WorkflowRunStatus::Failed, WorkflowNodeStatus::Failed)
            );
            assert_eq!(
                f.engine.resume_from_failure(&f.run_id).unwrap(),
                ora_application::ResumeWorkflowRunResult::Resumed
            );
            let replay = last_dispatch(&f.executor, "member");
            // A foreach member resolves its configuration per attempt from the revised graph.
            let revised = f
                .store
                .revised_graph(f.run_id.as_ref(), &replay.node_run_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                (
                    revised
                        .node("member")
                        .unwrap()
                        .agent_config
                        .as_ref()
                        .unwrap()
                        .prompt
                        .as_str(),
                    replay.variable_pool.values["iter.item"].as_str(),
                    dispatched(&f.executor, "member"),
                ),
                ("new", Some("b"), 3)
            );
            assert_eq!(
                dispatched(&f.executor, "member"),
                3,
                "item a must not run again"
            );
        }
    });
}
