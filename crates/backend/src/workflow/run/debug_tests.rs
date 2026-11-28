//! Reproducible stop → apply → re-dispatch scenarios (#1429) over real SQLite, the production
//! engine and controllable delayed agents (completion is test-driven). The first test proves the
//! happy path; the others pin idempotent confirmation, stale previews and failed quiescence.

use super::debug::{DebugSessionStopper, WorkflowDebugCoordinator};

mod reroute;
mod retirement;
use super::test_fixture::{RecordingExecutor, bootstrap, run_test, seeded_pending_run};
use super::transitions::WorkflowRunTransitions;
use crate::clock::SystemClock;
use crate::error::BackendError;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{
    UuidWorkflowNodeRunIdGenerator, WorkflowRunEngine, WorkflowRunEngineRepository,
    WorkflowRunInvalidationPublisher, WorkflowRunRepository, WorkflowVariablePool,
};
use ora_contracts::{CommitWorkflowDebugRequest, WorkflowDebugPhase, WorkflowDebugPreview};
use ora_db::workflow_debug::WorkflowDebugStore;
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{
    SessionId, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId,
    WorkflowRunStatus,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
mod lifecycle;
mod loop_exit_cleanup;
mod while_revisions;

mod foreach;

/// start → x → out, with an independent leaf branch start → s.
const GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"x","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"old"}}},
    {"id":"out","data":{"kind":"output","outputs":[{"name":"result","variableSelector":["x","output"]}]}},
    {"id":"s","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"independent"}}}
],"edges":[
    {"source":"start","target":"x"},
    {"source":"x","target":"out"},
    {"source":"start","target":"s"}
]}"#;

/// The happy-path attempts carry no live session host, so stopping needs no runtime.
struct NoSessions;
impl DebugSessionStopper for NoSessions {
    fn stop(
        &self,
        _session_id: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), BackendError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

/// A stop that cannot be proven: the coordinator must record stop_failed and keep the fence.
struct UnprovableStop;
impl DebugSessionStopper for UnprovableStop {
    fn stop(
        &self,
        _session_id: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), BackendError>> + Send + '_>> {
        Box::pin(async {
            Err(BackendError::internal(
                "stop",
                std::io::Error::new(std::io::ErrorKind::Other, "cannot prove isolation"),
            ))
        })
    }
}

/// Records every invalidation so tests can assert the notice follows the apply commit.
#[derive(Default)]
struct Invalidations(Mutex<Vec<String>>);
impl WorkflowRunInvalidationPublisher for Invalidations {
    fn publish_run_invalidated(&self, run_id: &WorkflowRunId) {
        self.0.lock().unwrap().push(run_id.to_string());
    }
}

fn dispatched(executor: &RecordingExecutor, node_id: &str) -> usize {
    executor
        .dispatches
        .lock()
        .unwrap()
        .iter()
        .filter(|id| *id == node_id)
        .count()
}

fn node_runs(pool: &RepositoryPool, run_id: &WorkflowRunId) -> Vec<WorkflowNodeRun> {
    SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(run_id)
        .unwrap()
}

/// The output node is swift: it completes inside a scheduling wave rather than through the async
/// executor, so its progress is asserted on persisted rows, not on dispatch records.
fn out_rows(pool: &RepositoryPool, run_id: &WorkflowRunId) -> Vec<WorkflowNodeRun> {
    node_runs(pool, run_id)
        .into_iter()
        .filter(|row| row.node_id == "out")
        .collect()
}

/// BackendError's public display is generic; the stable rejection reason lives in the source chain.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(inner) = source {
        message.push_str(" | ");
        message.push_str(&inner.to_string());
        source = inner.source();
    }
    message
}

/// A started run with the target x and the independent branch s in flight, dispatch evidence
/// recorded, and a coordinator wired to the same engine/gate/invalidations.
struct Fixture {
    _temp: tempfile::TempDir,
    pool: RepositoryPool,
    run_id: WorkflowRunId,
    executor: RecordingExecutor,
    engine: Arc<
        ora_application::WorkflowRunEngine<
            SqliteWorkflowRunEngineRepository,
            UuidWorkflowNodeRunIdGenerator,
            SystemClock,
        >,
    >,
    coordinator: WorkflowDebugCoordinator,
    store: WorkflowDebugStore,
    invalidations: Arc<Invalidations>,
}

impl Fixture {
    fn new(stopper: Arc<dyn DebugSessionStopper>) -> Self {
        Self::with_graph(stopper, GRAPH)
    }

    /// Starts a real graph so branch decisions and attempt evidence are recorded by dispatch.
    fn with_graph(stopper: Arc<dyn DebugSessionStopper>, graph: &str) -> Self {
        Self::with_inputs(stopper, graph, Default::default())
    }

    /// Commits named Start inputs before the first scheduling wave.
    fn with_inputs(
        stopper: Arc<dyn DebugSessionStopper>,
        graph: &str,
        inputs: std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Self {
        let (temp, pool) = bootstrap();
        let run_id = seeded_pending_run(&temp, &pool, graph);
        if !inputs.is_empty() {
            SqliteWorkflowRunEngineRepository::new(pool.clone())
                .update_run_input(&run_id, Some("kickoff".into()), inputs, 35)
                .unwrap();
        }
        let executor = RecordingExecutor::default();
        let run_locks = KeyedResourceLocks::new();
        let invalidations = Arc::new(Invalidations::default());
        let transitions = Arc::new(WorkflowRunTransitions::new(
            pool.clone(),
            invalidations.clone(),
            run_locks.clone(),
        ));
        let engine = Arc::new(WorkflowRunEngine::new(
            SqliteWorkflowRunEngineRepository::new(pool.clone()),
            executor.clone(),
            UuidWorkflowNodeRunIdGenerator::new(),
            SystemClock,
        ));
        engine.start(&run_id).unwrap();
        let store = WorkflowDebugStore::new(pool.clone());
        for row in node_runs(&pool, &run_id) {
            if row.node_id == "x" {
                SqliteWorkflowRunEngineRepository::new(pool.clone())
                    .bind_node_run_session(&row.id, &SessionId::new("session-1"), 50)
                    .unwrap();
            }
            if row.node_id == "x" || row.node_id == "s" {
                store
                    .record_agent_dispatch(
                        run_id.as_ref(),
                        row.id.as_ref(),
                        &json!({"variablePool": WorkflowVariablePool::default()}),
                    )
                    .unwrap();
            }
        }
        let coordinator = WorkflowDebugCoordinator::new(
            pool.clone(),
            run_locks,
            stopper,
            engine.clone(),
            transitions,
            SystemClock,
        );
        Self {
            _temp: temp,
            pool,
            run_id,
            executor,
            engine,
            coordinator,
            store,
            invalidations,
        }
    }

    fn row(&self, node_id: &str) -> WorkflowNodeRun {
        node_runs(&self.pool, &self.run_id)
            .into_iter()
            .find(|row| row.node_id == node_id)
            .unwrap()
    }

    /// Give a live test agent the same persisted stop handle as a dispatched session.
    fn bind_agent(&self, id: &WorkflowNodeRunId) {
        SqliteWorkflowRunEngineRepository::new(self.pool.clone())
            .bind_node_run_session(id, &SessionId::new("session-1"), 50)
            .unwrap();
    }

    fn preview(&self, node_run_id: &str, base_revision: u32) -> WorkflowDebugPreview {
        self.coordinator
            .preview(
                serde_json::from_value(json!({
                    "runId": self.run_id.as_ref(),
                    "nodeRunId": node_run_id,
                    "baseRevision": base_revision,
                    "patch": [{"op":"replace","path":"/agentConfig/prompt","value":"new"}]
                }))
                .unwrap(),
            )
            .unwrap()
    }

    fn commit_request(
        &self,
        preview: &WorkflowDebugPreview,
        request_id: &str,
    ) -> CommitWorkflowDebugRequest {
        CommitWorkflowDebugRequest {
            run_id: self.run_id.as_ref().to_string(),
            preview_id: preview.preview_id.clone(),
            request_id: request_id.to_string(),
        }
    }

    fn run_status(&self) -> WorkflowRunStatus {
        SqliteWorkflowRunRepository::new(self.pool.clone())
            .find_run(&self.run_id)
            .unwrap()
            .unwrap()
            .status
    }
}

#[test]
fn edit_third_round_member_preserves_feedback_and_dispatches_new_prompt() {
    for continue_after_edit in [false, true] {
        run_test(async {
            let graph = super::loop_resume_tests::LOOP_GRAPH
                .replace("\"maxIterations\":3", "\"maxIterations\":5");
            let f = Fixture::with_graph(Arc::new(NoSessions), &graph);
            for value in ["S1", "S2"] {
                let writer = node_runs(&f.pool, &f.run_id)
                    .into_iter()
                    .find(|r| r.node_id == "writer" && r.status == WorkflowNodeStatus::Running)
                    .unwrap();
                f.engine
                    .complete_node(
                        &f.run_id,
                        &writer.id,
                        Some(value.into()),
                        None,
                        None,
                        Vec::new(),
                    )
                    .unwrap();
            }
            let old = f.executor.records().pop().unwrap();
            let preview_request = || {
                serde_json::from_value(json!({
                    "runId": f.run_id.as_ref(), "nodeRunId": old.node_run_id, "baseRevision": 0,
                    "patch": [{"op":"replace","path":"/agentConfig/prompt","value":"edited"}],
                }))
                .unwrap()
            };
            let missing = f.coordinator.preview(preview_request()).unwrap_err();
            assert!(error_chain(&missing).contains("missing_input_evidence"));
            f.store
                .record_agent_dispatch(
                    f.run_id.as_ref(),
                    &old.node_run_id,
                    &json!({"variablePool":old.variable_pool}),
                )
                .unwrap();
            f.engine
                .complete_node(
                    &f.run_id,
                    &WorkflowNodeRunId::new(&old.node_run_id),
                    Some("old fourth input".into()),
                    None,
                    None,
                    Vec::new(),
                )
                .unwrap();
            let old_fourth = f.executor.records().pop().unwrap();
            f.bind_agent(&WorkflowNodeRunId::new(&old_fourth.node_run_id));
            let preview = f.coordinator.preview(preview_request()).unwrap();
            assert_eq!(preview.target.round, Some(3));
            assert!(preview.impact.invalidate.contains(&old.node_run_id));
            assert_eq!(preview.impact.stop, vec![old_fourth.node_run_id.clone()]);
            f.coordinator
                .commit(f.commit_request(&preview, "third"))
                .await
                .unwrap();
            let rows = f.executor.records();
            let writers: Vec<_> = rows
                .iter()
                .filter(|r| r.node_id == "writer")
                .map(|r| {
                    (
                        r.prompt.as_deref().unwrap(),
                        r.variable_pool.values["loop.draft"].as_str().unwrap(),
                    )
                })
                .collect();
            assert_eq!(
                writers,
                vec![
                    ("revise", "seed"),
                    ("revise", "S1"),
                    ("revise", "S2"),
                    ("revise", "old fourth input"),
                    ("edited", "S2")
                ]
            );
            let mut new = WorkflowNodeRunId::new(&rows.last().unwrap().node_run_id);
            if continue_after_edit {
                // Preserve the pending fourth slot across an older catalog opening this database.
                let catalog = ora_db::default_migration_catalog().unwrap();
                let migration = catalog.migration("0028").unwrap();
                let db =
                    rusqlite::Connection::open(f._temp.path().join("repository.sqlite3")).unwrap();
                db.execute_batch(&migration.down_sql()).unwrap();
                db.execute_batch(&migration.up_sql()).unwrap();
                f.engine
                    .complete_node(&f.run_id, &new, Some("NEXT".into()), None, None, Vec::new())
                    .unwrap();
                let next = f.executor.records().pop().unwrap();
                assert_eq!(next.prompt.as_deref(), Some("edited"));
                assert_eq!(next.variable_pool.values["loop.draft"], json!("NEXT"));
                new = WorkflowNodeRunId::new(next.node_run_id);
            }
            f.engine
                .complete_node(&f.run_id, &new, Some("done".into()), None, None, Vec::new())
                .unwrap();
            assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
            assert_eq!(
                dispatched(&f.executor, "writer"),
                5 + usize::from(continue_after_edit)
            );
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(f.row("loop").output.as_deref().unwrap())
                    .unwrap(),
                json!({"result":"done"})
            );
            let retired = f
                .store
                .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                    run_id: f.run_id.to_string(),
                    node_run_id: old_fourth.node_run_id,
                })
                .unwrap();
            assert!(retired.superseded);
        });
    }
}

#[test]
fn rewinding_a_completed_loop_replaces_its_consumed_export_while_a_sibling_runs() {
    run_test(async {
        let mut graph: serde_json::Value =
            serde_json::from_str(super::loop_resume_tests::LOOP_GRAPH).unwrap();
        // The old run exhausts four rounds successfully; the edit exits at round three.
        let config = &mut graph["nodes"][1]["data"]["loopConfig"];
        config["maxIterations"] = json!(4);
        config["maxIterationsBehavior"] = json!("succeed");
        graph["nodes"].as_array_mut().unwrap().push(json!({
            "id":"sibling", "data":{"kind":"agent", "agentConfig":{
                "executor":{"agentCli":"c","modelId":"m"}, "prompt":"independent"
            }}
        }));
        graph["edges"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source":"start","target":"sibling"}));
        let f = Fixture::with_graph(Arc::new(NoSessions), &graph.to_string());
        let sibling = f.row("sibling");
        let mut history = Vec::new();
        for output in ["S1", "S2", "S3", "OLD"] {
            let writer = f
                .executor
                .records()
                .into_iter()
                .rev()
                .find(|r| r.node_id == "writer")
                .unwrap();
            let id = WorkflowNodeRunId::new(writer.node_run_id);
            f.store
                .record_agent_dispatch(
                    f.run_id.as_ref(),
                    id.as_ref(),
                    &json!({"variablePool":writer.variable_pool}),
                )
                .unwrap();
            f.engine
                .complete_node(&f.run_id, &id, Some(output.into()), None, None, vec![])
                .unwrap();
            history.push(id);
        }
        let old = f.row("out");
        assert_eq!(old.output.as_deref(), Some(r#"{"draft":"OLD"}"#));
        assert_eq!(f.run_status(), WorkflowRunStatus::Running);
        let preview = f.preview(history[2].as_ref(), 0);
        assert!(preview.impact.invalidate.contains(&old.id.to_string()));
        assert!(preview.impact.keep.contains(&sibling.id.to_string()));
        assert!(preview.impact.stop.is_empty());
        f.coordinator
            .commit(f.commit_request(&preview, "replace-export"))
            .await
            .unwrap();
        assert!(out_rows(&f.pool, &f.run_id).is_empty());
        let writer = f.executor.records().pop().unwrap();
        assert_eq!(
            (
                writer.prompt.as_deref(),
                writer.variable_pool.values["loop.draft"].as_str()
            ),
            (Some("new"), Some("S2"))
        );
        let repository = SqliteWorkflowRunRepository::new(f.pool.clone());
        let run = repository.find_run(&f.run_id).unwrap().unwrap();
        let state: serde_json::Value = serde_json::from_str(run.state.as_deref().unwrap()).unwrap();
        let mut anchors = state["current_nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect::<Vec<_>>();
        anchors.sort_unstable();
        assert_eq!(anchors, ["loop", "sibling"]);
        let parent = f.row("loop");
        assert_eq!(
            (parent.status, parent.output, parent.finished_at),
            (WorkflowNodeStatus::Running, None, None)
        );
        let metadata: serde_json::Value =
            serde_json::from_str(parent.payload.as_deref().unwrap_or("{}")).unwrap();
        assert!(metadata.get("stop_reason").is_none());
        let payload: ora_application::WorkflowRunPayload =
            serde_json::from_str(run.payload.as_deref().unwrap()).unwrap();
        assert!(!payload.variable_pool.values.contains_key("loop.result"));
        f.engine
            .complete_node(
                &f.run_id,
                &WorkflowNodeRunId::new(writer.node_run_id),
                Some("done".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        let replacement = f.row("out");
        assert_ne!(replacement.id, old.id);
        assert_eq!(replacement.output.as_deref(), Some(r#"{"draft":"done"}"#));
        let retired = f
            .store
            .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                run_id: f.run_id.to_string(),
                node_run_id: old.id.to_string(),
            })
            .unwrap();
        assert!(retired.superseded);
        assert_eq!(retired.output, old.output);
        assert_eq!(f.row("sibling"), sibling);
        assert_eq!(
            (
                dispatched(&f.executor, "sibling"),
                dispatched(&f.executor, "writer")
            ),
            (1, 5)
        );
        f.engine
            .complete_node(
                &f.run_id,
                &sibling.id,
                Some("independent".into()),
                None,
                None,
                vec![],
            )
            .unwrap();
        let finished = repository.find_run(&f.run_id).unwrap().unwrap();
        assert_eq!(
            (finished.status, finished.output),
            (WorkflowRunStatus::Succeeded, replacement.output)
        );
    });
}

#[test]
fn editing_loop_condition_replaces_the_live_else_path_with_an_explicit_exit() {
    run_test(async {
        let f = Fixture::with_graph(
            Arc::new(NoSessions),
            &super::loop_exit_tests::graph(/*parallel*/ true),
        );
        let slow = f.row("slow");
        f.engine
            .complete_node(
                &f.run_id,
                &f.row("writer").id,
                Some("stop now".into()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        let again = f.row("again");
        f.bind_agent(&again.id);
        let request = json!({
            "runId": f.run_id.as_ref(), "nodeRunId": f.row("check").id, "baseRevision": 0,
            "patch": [{"op":"replace","path":"/cases/0/conditions/0/value","value":"stop now"}],
        });
        let preview = f
            .coordinator
            .preview(serde_json::from_value(request).unwrap())
            .unwrap();
        assert_eq!(preview.impact.predicted_branch.as_deref(), Some("done"));
        assert_eq!(preview.impact.stop, vec![again.id.to_string()]);
        f.coordinator
            .commit(f.commit_request(&preview, "exit"))
            .await
            .unwrap();
        assert_eq!(f.run_status(), WorkflowRunStatus::Succeeded);
        assert_eq!(dispatched(&f.executor, "again"), 1);
        assert_eq!(
            (f.row("slow").id, f.row("slow").status),
            (slow.id, WorkflowNodeStatus::Cancelled)
        );
        assert_eq!(dispatched(&f.executor, "writer"), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(f.row("loop").output.as_deref().unwrap())
                .unwrap(),
            json!({"result":"stop now"})
        );
    });
}

#[test]
fn editing_loop_limit_restarts_at_entry_and_retires_the_running_member() {
    run_test(async {
        let f = Fixture::with_graph(Arc::new(NoSessions), super::loop_resume_tests::LOOP_GRAPH);
        let start = f.row("start");
        let old_parent = f.row("loop");
        let writer = f.row("writer");
        f.bind_agent(&writer.id);
        let preview = f.coordinator.preview(serde_json::from_value(json!({
            "runId": f.run_id.as_ref(), "nodeRunId": old_parent.id.as_ref(), "baseRevision": 0,
            "patch": [{"op":"replace", "path":"/loopConfig/maxIterations", "value":2}]
        })).unwrap()).unwrap();
        assert!(preview.impact.keep.contains(&start.id.to_string()));
        assert!(preview.impact.stop.contains(&writer.id.to_string()));
        let operation = f
            .coordinator
            .commit(f.commit_request(&preview, "limit"))
            .await
            .unwrap();
        assert_eq!(operation.phase, WorkflowDebugPhase::Applied { revision: 1 });
        assert_ne!(old_parent.id, f.row("loop").id);
        assert_ne!(writer.scope_id, f.row("writer").scope_id);
        for count in [2, 3] {
            assert_eq!(dispatched(&f.executor, "writer"), count);
            let row = node_runs(&f.pool, &f.run_id)
                .into_iter()
                .find(|r| r.node_id == "writer" && r.status == WorkflowNodeStatus::Running)
                .unwrap();
            f.engine
                .complete_node(
                    &f.run_id,
                    &row.id,
                    Some("again".into()),
                    None,
                    None,
                    Vec::new(),
                )
                .unwrap();
        }
        assert_eq!(dispatched(&f.executor, "writer"), 3);
        assert_eq!(f.run_status(), WorkflowRunStatus::Failed);
        assert_eq!(f.row("start").id, start.id);
    });
}

/// A parked Loop member is fenced as soon as its upstream edit is claimed and retired on apply.
#[test]
fn editing_upstream_of_a_loop_retires_its_parked_member() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let (upstream, writer) = super::test_fixture::parked_writer_after_upstream(&temp, &pool);
        assert_eq!(writer.status, WorkflowNodeStatus::Pending);
        let store = WorkflowDebugStore::new(pool.clone());
        let request = serde_json::from_value(json!({
            "runId":writer.run_id, "nodeRunId":upstream, "baseRevision":0,
            "patch":[{"op":"replace","path":"/agentConfig/prompt","value":"revised"}]
        }))
        .unwrap();
        let preview = store
            .preview(&request, super::debug::strategy::plan)
            .unwrap();
        assert!(preview.impact.invalidate.contains(&writer.id.to_string()));
        assert!(preview.impact.stop.is_empty());
        let run = writer.run_id.as_ref();
        store
            .claim(&CommitWorkflowDebugRequest {
                run_id: run.into(),
                preview_id: preview.preview_id,
                request_id: "edit".into(),
            })
            .unwrap();
        assert!(
            store
                .dispatch_fence(run)
                .unwrap()
                .unwrap()
                .affected_attempts
                .contains(&writer.id)
        );
        let (locks, completing) = super::test_fixture::locks();
        let (transitions, _) = super::test_fixture::recording_transitions(&pool);
        let prompt = || {
            super::interactive::begin_human_turn(
                &pool,
                &locks,
                &completing,
                &transitions,
                "session-1",
            )
        };
        let rejected = ora_contracts::PublicError::WorkflowNodeNotAwaitingInput(
            ora_contracts::EmptyErrorParams {},
        );
        assert_eq!(prompt().await.unwrap_err().public_error(), &rejected);
        let repo = SqliteWorkflowRunRepository::new(pool.clone());
        assert!(
            repo.list_node_runs(&writer.run_id)
                .unwrap()
                .contains(&writer)
        );
        assert_eq!(
            store
                .apply_after_stop(run, "edit", 61, |tx, p| super::debug::strategy::settle(
                    tx, run, 61, p
                ))
                .unwrap()
                .phase,
            WorkflowDebugPhase::Applied { revision: 1 }
        );
        let detail = repo.get_run_detail(&writer.run_id).unwrap().unwrap();
        assert!(!detail.nodes.iter().any(|row| row.id == writer.id));
        assert_eq!(
            detail
                .scopes
                .iter()
                .find(|scope| scope.id == writer.scope_id)
                .unwrap()
                .status,
            ora_domain::WorkflowScopeStatus::Cancelled
        );
        assert!(
            store
                .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                    run_id: run.into(),
                    node_run_id: writer.id.to_string()
                })
                .unwrap()
                .superseded
        );
    });
}

#[test]
fn confirmed_edit_stops_only_the_target_and_redispatches_under_the_new_revision() {
    run_test(async {
        let f = Fixture::new(Arc::new(NoSessions));
        let old_x = f.row("x");
        let s = f.row("s");
        assert_eq!(old_x.status, WorkflowNodeStatus::Running);
        assert_eq!(s.status, WorkflowNodeStatus::Running);
        assert_eq!(
            (dispatched(&f.executor, "x"), dispatched(&f.executor, "s")),
            (1, 1)
        );

        let preview = f.preview(old_x.id.as_ref(), 0);
        // Only the target's attempt is stopped; the independent branch (and start) are kept.
        assert_eq!(preview.impact.stop, vec![old_x.id.to_string()]);
        assert!(preview.impact.keep.contains(&s.id.to_string()));
        assert!(!preview.impact.keep.contains(&old_x.id.to_string()));
        assert!(preview.impact.schedule.contains(&"x".to_string()));
        assert!(preview.impact.schedule.contains(&"out".to_string()));

        let invalidations_before = f.invalidations.0.lock().unwrap().len();
        let operation = f
            .coordinator
            .commit(f.commit_request(&preview, "confirm"))
            .await
            .unwrap();
        assert_eq!(operation.phase, WorkflowDebugPhase::Applied { revision: 1 });

        // The revision applied and the notice published before the re-dispatch wave.
        assert!(f.invalidations.0.lock().unwrap().len() > invalidations_before);
        // The target re-dispatched exactly once; the independent branch was not re-dispatched.
        assert_eq!(
            (dispatched(&f.executor, "x"), dispatched(&f.executor, "s")),
            (2, 1)
        );
        assert!(out_rows(&f.pool, &f.run_id).is_empty());

        // The old attempt is retired; the new attempt is bound to revision 1; the sibling still runs.
        let new_x = f.row("x");
        assert_eq!(new_x.status, WorkflowNodeStatus::Running);
        assert_ne!(new_x.id, old_x.id);
        let retired = f
            .store
            .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                run_id: f.run_id.as_ref().to_string(),
                node_run_id: old_x.id.to_string(),
            })
            .unwrap();
        assert!(retired.superseded);
        assert_eq!(
            retired.retired_by_operation_id,
            Some(operation.operation_id.clone())
        );
        let new_attempt = f
            .store
            .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                run_id: f.run_id.as_ref().to_string(),
                node_run_id: new_x.id.to_string(),
            })
            .unwrap();
        assert_eq!(new_attempt.revision, Some(1));
        assert_eq!(f.row("s").status, WorkflowNodeStatus::Running);

        // Release the old agent: its late return against the retired attempt is a no-op and must
        // not overwrite the new result or dispatch the downstream a second time.
        f.engine
            .complete_node(
                &f.run_id,
                &old_x.id,
                Some("old result".into()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        assert!(out_rows(&f.pool, &f.run_id).is_empty());
        assert_eq!(dispatched(&f.executor, "x"), 2);

        // The new attempt completes; the downstream reads only the new result; the sibling finishes.
        f.engine
            .complete_node(
                &f.run_id,
                &new_x.id,
                Some("new result".into()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        f.engine
            .complete_node(
                &f.run_id,
                &s.id,
                Some("s done".into()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        let run = SqliteWorkflowRunRepository::new(f.pool.clone())
            .find_run(&f.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.status, WorkflowRunStatus::Succeeded);
        assert_eq!(run.output.as_deref(), Some(r#"{"result":"new result"}"#));
        let out = out_rows(&f.pool, &f.run_id);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, WorkflowNodeStatus::Succeeded);
    });
}

/// A second edit reverses the first one: neither old branch output can win the join.
#[test]
fn rerouting_twice_retracts_old_values_before_the_join() {
    run_test(async {
        let graph = r#"{"nodes":[
            {"id":"start","data":{"kind":"start","inputVariables":[{"name":"flag","valueType":"boolean"}]}},
            {"id":"c","data":{"kind":"condition","cases":[{"id":"yes","logic":"and","conditions":[{"variableSelector":["start","flag"],"operator":"is","value":true}]}]}},
            {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
            {"id":"b","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
            {"id":"s","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"s"}}},
            {"id":"j","data":{"kind":"aggregator","aggregatorConfig":{"variables":[["a","output"],["b","output"]]}}}
        ],"edges":[{"source":"start","target":"c"},{"source":"start","target":"s"},{"source":"c","sourceHandle":"yes","target":"a"},{"source":"c","sourceHandle":"else","target":"b"},{"source":"a","target":"j"},{"source":"b","target":"j"}]}"#;
        let f = Fixture::with_inputs(
            Arc::new(NoSessions),
            graph,
            [("flag".into(), json!(true))].into(),
        );
        let start = f.row("start").id;
        for (revision, old, selected, value) in [(0, "a", "b", false), (1, "b", "a", true)] {
            let row = f.row(old);
            f.engine
                .complete_node(&f.run_id, &row.id, Some(old.into()), None, None, Vec::new())
                .unwrap();
            assert_eq!(f.row("j").status, WorkflowNodeStatus::Succeeded);
            let run = SqliteWorkflowRunRepository::new(f.pool.clone())
                .find_run(&f.run_id)
                .unwrap()
                .unwrap();
            let payload: ora_application::WorkflowRunPayload =
                serde_json::from_str(run.payload.as_deref().unwrap()).unwrap();
            assert_eq!(
                payload.variable_pool.values.get("j.output"),
                Some(&json!(old))
            );
            let preview = f.coordinator.preview(serde_json::from_value(json!({"runId":f.run_id.as_ref(),"nodeRunId":f.row("c").id.to_string(),"baseRevision":revision,"patch":[{"op":"replace","path":"/cases/0/conditions/0/value","value":value}]})).unwrap()).unwrap();
            assert!(preview.impact.invalidate.contains(&row.id.to_string()));
            assert_eq!(preview.impact.schedule, vec!["c", selected, "j"]);
            f.coordinator
                .commit(f.commit_request(&preview, selected))
                .await
                .unwrap();
            assert_eq!(f.row(selected).status, WorkflowNodeStatus::Running);
            assert!(
                !node_runs(&f.pool, &f.run_id)
                    .iter()
                    .any(|r| r.node_id == "j")
            );
            let history = f
                .store
                .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                    run_id: f.run_id.to_string(),
                    node_run_id: row.id.to_string(),
                })
                .unwrap();
            assert!(history.superseded);
        }
        assert_eq!(start, f.row("start").id);
        assert_eq!(
            (
                dispatched(&f.executor, "a"),
                dispatched(&f.executor, "b"),
                dispatched(&f.executor, "s")
            ),
            (2, 1, 1)
        );
    });
}

/// A logical position is not an escape hatch around the ordinary predecessor gate.
#[test]
fn pending_agent_behind_running_predecessor_has_no_preview() {
    let graph = GRAPH.replace(
        r#"{"source":"start","target":"s"}"#,
        r#"{"source":"x","target":"s"}"#,
    );
    let f = Fixture::with_graph(Arc::new(NoSessions), &graph);
    let root = f.row("start").scope_id.to_string();
    let request = serde_json::from_value(json!({
        "runId": f.run_id.as_ref(), "baseRevision":0,
        "pendingTarget":{"rootScopeId":root,"scopeId":root,"nodeId":"s"},
        "patch":[{"op":"replace","path":"/agentConfig/prompt","value":"edited"}]
    }))
    .unwrap();
    let error = f.coordinator.preview(request).unwrap_err();
    assert!(error_chain(&error).contains("prerequisites"), "{error}");
    assert!(f.store.history(f.run_id.as_ref()).unwrap().is_empty());
    assert_eq!(dispatched(&f.executor, "s"), 0);
}

#[test]
fn duplicate_confirmation_reads_the_same_operation_without_a_second_dispatch() {
    run_test(async {
        let f = Fixture::new(Arc::new(NoSessions));
        let old_x = f.row("x");
        let preview = f.preview(old_x.id.as_ref(), 0);
        let request = f.commit_request(&preview, "same-key");

        let first = f.coordinator.commit(request.clone()).await.unwrap();
        assert_eq!(first.phase, WorkflowDebugPhase::Applied { revision: 1 });
        let dispatches_after_first = dispatched(&f.executor, "x");

        // A reconnect/duplicate with the same key and credential reads the same operation back and
        // starts nothing new: no second revision, no second dispatch.
        let second = f.coordinator.commit(request.clone()).await.unwrap();
        assert_eq!(second, first);
        assert_eq!(dispatched(&f.executor, "x"), dispatches_after_first);
        assert_eq!(f.store.history(f.run_id.as_ref()).unwrap().len(), 1);

        // The same key with a different credential is a conflict, not a new operation.
        let new_x = f.row("x");
        f.store
            .record_agent_dispatch(
                f.run_id.as_ref(),
                new_x.id.as_ref(),
                &json!({"variablePool": WorkflowVariablePool::default()}),
            )
            .unwrap();
        let next_preview = f.preview(new_x.id.as_ref(), 1);
        let conflict = f.commit_request(&next_preview, "same-key");
        let error = f.coordinator.commit(conflict).await.unwrap_err();
        assert!(error_chain(&error).contains("request_conflict"), "{error}");
        assert_eq!(f.store.history(f.run_id.as_ref()).unwrap().len(), 1);
    });
}

#[test]
fn a_changed_input_before_confirmation_returns_stale_preview_with_no_side_effects() {
    run_test(async {
        let f = Fixture::new(Arc::new(NoSessions));
        let old_x = f.row("x");
        let preview = f.preview(old_x.id.as_ref(), 0);

        // Normal scheduling completes the target before the confirmation lands: the read set the
        // preview froze no longer matches, so the confirmation must be refused without stopping or
        // applying anything.
        f.engine
            .complete_node(
                &f.run_id,
                &old_x.id,
                Some("done".into()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        let error = f
            .coordinator
            .commit(f.commit_request(&preview, "late"))
            .await
            .unwrap_err();
        assert!(error_chain(&error).contains("stale_preview"), "{error}");
        assert!(f.store.history(f.run_id.as_ref()).unwrap().is_empty());
        assert_eq!(dispatched(&f.executor, "x"), 1);
    });
}

#[test]
fn an_unprovable_stop_records_stop_failed_and_keeps_the_fence_while_the_sibling_finishes() {
    run_test(async {
        let f = Fixture::new(Arc::new(UnprovableStop));
        let old_x = f.row("x");
        // Bind a session so the coordinator actually attempts a stop (which cannot be proven).
        let repository = SqliteWorkflowRunEngineRepository::new(f.pool.clone());
        repository
            .bind_node_run_session(&old_x.id, &SessionId::new("session-1"), 50)
            .unwrap();
        let preview = f.preview(old_x.id.as_ref(), 0);

        let operation = f
            .coordinator
            .commit(f.commit_request(&preview, "fail-stop"))
            .await
            .unwrap();
        assert!(matches!(
            operation.phase,
            WorkflowDebugPhase::StopFailed { .. }
        ));

        // Zero new revision and zero new dispatch in the affected scope; the fence stays, so a fresh
        // preview is refused while the failed stop is unreleased.
        assert_eq!(dispatched(&f.executor, "x"), 1);
        assert!(
            f.coordinator
                .preview(
                    serde_json::from_value(json!({
                        "runId": f.run_id.as_ref(),
                        "nodeRunId": old_x.id.as_ref(),
                        "baseRevision": 0,
                        "patch": [{"op":"replace","path":"/agentConfig/prompt","value":"again"}]
                    }))
                    .unwrap()
                )
                .is_err()
        );

        // The independent branch is untouched by the fence and can still commit its result.
        let s = f.row("s");
        f.engine
            .complete_node(
                &f.run_id,
                &s.id,
                Some("s done".into()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        assert_eq!(f.row("s").status, WorkflowNodeStatus::Succeeded);
        // The run does not finish while the failed fence is unreleased.
        assert_eq!(f.run_status(), WorkflowRunStatus::Running);
        let history = f.store.history(f.run_id.as_ref()).unwrap();
        assert_eq!(history.len(), 1);
        assert!(matches!(
            history[0].operation.phase,
            WorkflowDebugPhase::StopFailed { .. }
        ));

        // Normal scheduling entry points are refused while the fence holds, and admitted once the
        // failed stop is released after isolation is independently proven.
        assert!(f.coordinator.ensure_no_fence(f.run_id.as_ref()).is_err());
        f.store
            .release_failed_stop(f.run_id.as_ref(), "fail-stop")
            .unwrap();
        assert!(f.coordinator.ensure_no_fence(f.run_id.as_ref()).is_ok());
    });
}
