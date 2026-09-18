use super::test_fixture::{ClockAt, RecordingExecutor, SeqGen, bootstrap, started_run_with};
use ora_application::{
    NodeFailure, NodeFailureKind, ResumeWorkflowRunResult, WorkflowRunEngine,
    WorkflowRunEngineRepository, WorkflowRunRepository,
};
use ora_db::{SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::params;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

const U1_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"b","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"c"}}},
    {"id":"d","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"d"}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"start","target":"b"},
    {"source":"a","target":"c"},
    {"source":"b","target":"c"},
    {"source":"c","target":"d"},
    {"source":"d","target":"output"}
]}"#;

const U2_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"b","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"c"}}},
    {"id":"d","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"d"}}},
    {"id":"out_c","data":{"kind":"output"}},
    {"id":"out_d","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"start","target":"b"},
    {"source":"a","target":"c"},
    {"source":"b","target":"c"},
    {"source":"b","target":"d"},
    {"source":"c","target":"out_c"},
    {"source":"d","target":"out_d"}
]}"#;

fn live_nodes(pool: &ora_db::RepositoryPool, run_id: &WorkflowRunId) -> Vec<WorkflowNodeRun> {
    SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(run_id)
        .unwrap()
}

fn live<'a>(nodes: &'a [WorkflowNodeRun], node_id: &str) -> &'a WorkflowNodeRun {
    nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .unwrap_or_else(|| panic!("missing live node {node_id}"))
}

fn dispatch_counts(executor: &RecordingExecutor) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for node_id in executor.dispatches.lock().expect("dispatch log").iter() {
        *counts.entry(node_id.clone()).or_insert(0) += 1;
    }
    counts
}

fn deleted_count(temp: &TempDir, run_id: &str, node_id: &str) -> i64 {
    let path = temp.path().join("repository.sqlite3");
    query_count(&path, run_id, node_id, 1)
}

fn live_count(temp: &TempDir, run_id: &str, node_id: &str) -> i64 {
    let path = temp.path().join("repository.sqlite3");
    query_count(&path, run_id, node_id, 0)
}

fn query_count(path: &Path, run_id: &str, node_id: &str, is_deleted: i64) -> i64 {
    let connection = rusqlite::Connection::open(path).unwrap();
    connection.busy_timeout(Duration::from_secs(5)).unwrap();
    connection
        .query_row(
            "SELECT COUNT(*) FROM workflow_node_runs
             WHERE run_id = ?1 AND node_id = ?2 AND is_deleted = ?3",
            params![run_id, node_id, is_deleted],
            |row| row.get(0),
        )
        .unwrap()
}

fn find_run(pool: &ora_db::RepositoryPool, run_id: &WorkflowRunId) -> ora_domain::WorkflowRun {
    SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(run_id)
        .unwrap()
        .unwrap()
}

type FixtureEngine = WorkflowRunEngine<SqliteWorkflowRunEngineRepository, SeqGen, ClockAt>;

fn complete(
    engine: &FixtureEngine,
    run_id: &WorkflowRunId,
    node_run_id: &ora_domain::WorkflowNodeRunId,
) {
    engine
        .complete_node(
            run_id,
            node_run_id,
            Some("ok".to_string()),
            None,
            None,
            Vec::new(),
        )
        .unwrap();
}

/// U1: succeeded siblings stay, the failed join node is re-dispatched, then the run finishes.
#[test]
fn resume_from_failure_redoes_failed_join_and_finishes() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, U1_GRAPH, executor.clone());
        let nodes = live_nodes(&pool, &run_id);
        let a_id = live(&nodes, "a").id.clone();
        let b_id = live(&nodes, "b").id.clone();
        complete(&engine, &run_id, &a_id);
        complete(&engine, &run_id, &b_id);
        let after_ab = live_nodes(&pool, &run_id);
        let c_old = live(&after_ab, "c").id.clone();
        engine
            .fail_node(
                &run_id,
                &c_old,
                NodeFailure::new(NodeFailureKind::Session, "c failed"),
            )
            .unwrap();

        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );

        let after = live_nodes(&pool, &run_id);
        let a = live(&after, "a");
        let b = live(&after, "b");
        assert_eq!(a.id, a_id);
        assert_eq!(b.id, b_id);
        assert_eq!(a.audit_fields.is_deleted, false);
        assert_eq!(b.audit_fields.is_deleted, false);
        let c_new = live(&after, "c");
        assert_ne!(c_new.id, c_old);
        assert_eq!(c_new.status, WorkflowNodeStatus::Running);
        assert_eq!(deleted_count(&temp, run_id.as_ref(), "c"), 1);
        let run = find_run(&pool, &run_id);
        assert_eq!(run.status, WorkflowRunStatus::Running);
        assert_eq!(run.error, None);
        let counts = dispatch_counts(&executor);
        assert_eq!(counts.get("a"), Some(&1));
        assert_eq!(counts.get("b"), Some(&1));
        assert_eq!(counts.get("c"), Some(&2));

        complete(&engine, &run_id, &c_new.id);
        let after_c = live_nodes(&pool, &run_id);
        let d = live(&after_c, "d").id.clone();
        complete(&engine, &run_id, &d);
        let finished = find_run(&pool, &run_id);
        assert_eq!(finished.status, WorkflowRunStatus::Succeeded);
    });
}

/// U2 / 0.4: a late success after failure must not dispatch dependents; resume redoes only A.
#[test]
fn failed_run_does_not_dispatch_on_late_success_then_resume_clears_only_failed_branch() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, U2_GRAPH, executor.clone());
        let nodes = live_nodes(&pool, &run_id);
        let a_id = live(&nodes, "a").id.clone();
        let b_id = live(&nodes, "b").id.clone();
        engine
            .fail_node(
                &run_id,
                &a_id,
                NodeFailure::new(NodeFailureKind::Session, "error-a"),
            )
            .unwrap();
        complete(&engine, &run_id, &b_id);

        let after_late = live_nodes(&pool, &run_id);
        assert_eq!(live(&after_late, "b").status, WorkflowNodeStatus::Succeeded);
        assert!(after_late.iter().all(|node| node.node_id != "c"));
        assert!(after_late.iter().all(|node| node.node_id != "d"));
        assert_eq!(live_count(&temp, run_id.as_ref(), "c"), 0);
        assert_eq!(live_count(&temp, run_id.as_ref(), "d"), 0);
        let failed = find_run(&pool, &run_id);
        assert_eq!(failed.status, WorkflowRunStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("error-a"));

        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let after_resume = live_nodes(&pool, &run_id);
        assert_eq!(live(&after_resume, "b").id, b_id);
        assert_eq!(
            live(&after_resume, "b").status,
            WorkflowNodeStatus::Succeeded
        );
        assert_ne!(live(&after_resume, "a").id, a_id);
        assert_eq!(live(&after_resume, "a").status, WorkflowNodeStatus::Running);
        assert_eq!(live(&after_resume, "d").status, WorkflowNodeStatus::Running);
        assert!(after_resume.iter().all(|node| node.node_id != "c"));
        assert_eq!(deleted_count(&temp, run_id.as_ref(), "a"), 1);
        let counts = dispatch_counts(&executor);
        assert_eq!(counts.get("a"), Some(&2));
        assert_eq!(counts.get("b"), Some(&1));
        assert_eq!(counts.get("d"), Some(&1));
        assert_eq!(counts.get("c"), None);
    });
}

/// U3: crash recovery marks the in-flight node failed, then resume behaves like U1.
#[test]
fn resume_from_failure_after_orphaned_crash_recovery() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, U1_GRAPH, executor.clone());
        let nodes = live_nodes(&pool, &run_id);
        complete(&engine, &run_id, &live(&nodes, "a").id);
        complete(&engine, &run_id, &live(&nodes, "b").id);
        let after_ab = live_nodes(&pool, &run_id);
        let c_old = live(&after_ab, "c").id.clone();
        SqliteWorkflowRunEngineRepository::new(pool.clone())
            .fail_orphaned_node_runs(&[run_id.clone()], 80)
            .unwrap();
        let after_crash = live_nodes(&pool, &run_id);
        let crashed = live(&after_crash, "c");
        assert_eq!(crashed.status, WorkflowNodeStatus::Failed);
        assert!(
            crashed
                .error
                .as_deref()
                .is_some_and(|error| error.contains("interrupted_by_restart"))
        );
        let run = find_run(&pool, &run_id);
        assert_eq!(run.status, WorkflowRunStatus::Failed);

        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let after_resume = live_nodes(&pool, &run_id);
        let c_new = live(&after_resume, "c");
        assert_ne!(c_new.id, c_old);
        complete(&engine, &run_id, &c_new.id);
        let after_c = live_nodes(&pool, &run_id);
        let d = live(&after_c, "d").id.clone();
        complete(&engine, &run_id, &d);
        assert_eq!(
            find_run(&pool, &run_id).status,
            WorkflowRunStatus::Succeeded
        );
    });
}
