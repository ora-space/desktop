//! The first reproducible stop → apply → re-dispatch scenario (#1429): a confirmed top-level edit
//! quiesces only the target's old attempt, keeps the independent branch running, applies the new
//! revision in one commit, notifies, and re-dispatch the target exactly once. A late return from the
//! old attempt must not overwrite the new result or trigger an extra downstream dispatch.

use super::debug::{DebugSessionStopper, WorkflowDebugCoordinator};
use super::test_fixture::{RecordingExecutor, bootstrap, run_test, seeded_pending_run};
use super::transitions::WorkflowRunTransitions;
use crate::clock::SystemClock;
use crate::error::BackendError;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{WorkflowRunEngine, WorkflowRunInvalidationPublisher, WorkflowRunRepository};
use ora_contracts::{CommitWorkflowDebugRequest, WorkflowDebugPhase};
use ora_db::workflow_debug::WorkflowDebugStore;
use ora_db::{SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

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

/// The scenario's attempts carry no live session host, so stopping needs no runtime; the fence,
/// apply and re-dispatch are what this test proves.
struct NoSessions;
impl DebugSessionStopper for NoSessions {
    fn stop(
        &self,
        _session_id: String,
    ) -> Pin<Box<dyn Future<Output = Result<(), BackendError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
}

/// Records every invalidation so the test can assert the notice follows the apply commit.
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

fn node_runs(
    pool: &ora_db::RepositoryPool,
    run_id: &WorkflowRunId,
) -> Vec<ora_domain::WorkflowNodeRun> {
    SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(run_id)
        .unwrap()
}

/// The output node is swift: it completes inside a scheduling wave rather than through the async
/// executor, so its progress is asserted on persisted rows, not on dispatch records.
fn out_rows(
    pool: &ora_db::RepositoryPool,
    run_id: &WorkflowRunId,
) -> Vec<ora_domain::WorkflowNodeRun> {
    node_runs(pool, run_id)
        .into_iter()
        .filter(|row| row.node_id == "out")
        .collect()
}

#[test]
fn confirmed_edit_stops_only_the_target_and_redispatches_under_the_new_revision() {
    run_test(async {
        let (temp, pool) = bootstrap();
        let run_id = seeded_pending_run(&temp, &pool, GRAPH);
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
            ora_application::UuidWorkflowNodeRunIdGenerator::new(),
            SystemClock,
        ));
        engine.start(&run_id).unwrap();

        // The target x and the independent branch s are both in flight (completion is test-driven).
        let rows = node_runs(&pool, &run_id);
        let old_x = rows.iter().find(|row| row.node_id == "x").unwrap();
        let s = rows.iter().find(|row| row.node_id == "s").unwrap();
        assert_eq!(old_x.status, WorkflowNodeStatus::Running);
        assert_eq!(s.status, WorkflowNodeStatus::Running);
        assert_eq!(
            (dispatched(&executor, "x"), dispatched(&executor, "s")),
            (1, 1)
        );

        // Dispatch evidence the real session driver would record at the prompt handoff.
        let store = WorkflowDebugStore::new(pool.clone());
        for row in [old_x, s] {
            store
                .record_agent_dispatch(
                    run_id.as_ref(),
                    row.id.as_ref(),
                    &json!({"variablePool": ora_application::WorkflowVariablePool::default()}),
                )
                .unwrap();
        }

        let coordinator = WorkflowDebugCoordinator::new(
            pool.clone(),
            run_locks.clone(),
            Arc::new(NoSessions),
            engine.clone(),
            transitions.clone(),
            SystemClock,
        );
        let preview = coordinator
            .preview(
                serde_json::from_value(json!({
                    "runId": run_id.as_ref(),
                    "nodeRunId": old_x.id.as_ref(),
                    "baseRevision": 0,
                    "patch": [{"op":"replace","path":"/agentConfig/prompt","value":"new"}]
                }))
                .unwrap(),
            )
            .unwrap();
        // Only the target's attempt is stopped; the independent branch (and start) are kept.
        assert_eq!(preview.impact.stop, vec![old_x.id.to_string()]);
        assert!(preview.impact.keep.contains(&s.id.to_string()));
        assert!(!preview.impact.keep.contains(&old_x.id.to_string()));
        assert!(preview.impact.schedule.contains(&"x".to_string()));
        assert!(preview.impact.schedule.contains(&"out".to_string()));

        let invalidations_before = invalidations.0.lock().unwrap().len();
        let operation = coordinator
            .commit(CommitWorkflowDebugRequest {
                run_id: run_id.as_ref().to_string(),
                preview_id: preview.preview_id.clone(),
                request_id: "confirm".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(operation.phase, WorkflowDebugPhase::Applied { revision: 1 });

        // The revision applied and the notice published before the re-dispatch wave.
        assert!(invalidations.0.lock().unwrap().len() > invalidations_before);
        // The target re-dispatched exactly once; the independent branch was not re-dispatched.
        assert_eq!(
            (dispatched(&executor, "x"), dispatched(&executor, "s")),
            (2, 1)
        );
        assert!(out_rows(&pool, &run_id).is_empty());

        // The old attempt is retired; the new attempt is bound to revision 1; the sibling still runs.
        let rows = node_runs(&pool, &run_id);
        let new_x = rows
            .iter()
            .find(|row| row.node_id == "x" && row.status == WorkflowNodeStatus::Running)
            .unwrap();
        assert_ne!(new_x.id, old_x.id);
        let retired = store
            .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                run_id: run_id.as_ref().to_string(),
                node_run_id: old_x.id.to_string(),
            })
            .unwrap();
        assert!(retired.superseded);
        assert_eq!(
            retired.retired_by_operation_id,
            Some(operation.operation_id.clone())
        );
        let new_attempt = store
            .attempt(&ora_contracts::GetWorkflowDebugAttemptRequest {
                run_id: run_id.as_ref().to_string(),
                node_run_id: new_x.id.to_string(),
            })
            .unwrap();
        assert_eq!(new_attempt.revision, Some(1));
        let s_now = rows.iter().find(|row| row.node_id == "s").unwrap();
        assert_eq!(s_now.status, WorkflowNodeStatus::Running);

        // Release the old agent: its late return against the retired attempt is a no-op and must
        // not overwrite the new result or dispatch the downstream a second time.
        engine
            .complete_node(
                &run_id,
                &old_x.id,
                Some("old result".to_string()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        assert!(out_rows(&pool, &run_id).is_empty());
        assert_eq!(dispatched(&executor, "x"), 2);

        // The new attempt completes; the downstream reads only the new result; the sibling finishes.
        engine
            .complete_node(
                &run_id,
                &new_x.id,
                Some("new result".to_string()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        engine
            .complete_node(
                &run_id,
                &s.id,
                Some("s done".to_string()),
                None,
                None,
                Vec::new(),
            )
            .unwrap();
        let run = SqliteWorkflowRunRepository::new(pool.clone())
            .find_run(&run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.status, WorkflowRunStatus::Succeeded);
        assert_eq!(run.output.as_deref(), Some(r#"{"result":"new result"}"#));
        let out = out_rows(&pool, &run_id);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, WorkflowNodeStatus::Succeeded);
    });
}
