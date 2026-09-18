//! Resume, cancel, and checkpoint behaviour of Loop containers through the production
//! scheduler and repository: a Loop node is one resume unit, and a rerun starts from round 1.

use super::test_fixture::{ClockAt, RecordingExecutor, SeqGen, bootstrap, started_run_with};
use ora_application::{
    CancelWorkflowRunResult, NodeFailure, NodeFailureKind, ResumeWorkflowRunResult,
    WorkflowRunEngine, WorkflowRunRepository,
};
use ora_db::{SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{
    WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus,
    WorkflowScopeStatus,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::params;
use std::collections::BTreeMap;
use tempfile::TempDir;

/// One Loop whose body is `entry → writer`; the loop ends once the writer answers `done`.
const LOOP_GRAPH: &str = r#"{"schemaVersion":2,"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"loop","data":{"kind":"loop","loopConfig":{
        "maxIterations":3,
        "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":"seed"},"feedback":["writer","output"]}],
        "until":{"logic":"and","conditions":[{"variableSelector":["writer","output"],"operator":"equals","value":"done"}]},
        "outputs":[{"name":"result","variableSelector":["writer","output"]}]
    }}},
    {"id":"entry","parentId":"loop","data":{"kind":"start","containerId":"loop"}},
    {"id":"writer","parentId":"loop","data":{"kind":"agent","containerId":"loop","agentConfig":{
        "executor":{"agentCli":"c","modelId":"m"},"prompt":"revise"
    }}},
    {"id":"out","data":{"kind":"output","outputs":[{"name":"draft","variableSelector":["loop","result"]}]}}
],"edges":[
    {"source":"start","target":"loop"},
    {"source":"entry","target":"writer"},
    {"source":"loop","target":"out"}
]}"#;

/// The same Loop next to an ordinary agent sibling `a`; both feed the output node.
const LOOP_WITH_SIBLING_GRAPH: &str = r#"{"schemaVersion":2,"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"loop","data":{"kind":"loop","loopConfig":{
        "maxIterations":3,
        "variables":[{"name":"draft","valueType":"string","initial":{"kind":"constant","value":"seed"},"feedback":["writer","output"]}],
        "until":{"logic":"and","conditions":[{"variableSelector":["writer","output"],"operator":"equals","value":"done"}]},
        "outputs":[{"name":"result","variableSelector":["writer","output"]}]
    }}},
    {"id":"entry","parentId":"loop","data":{"kind":"start","containerId":"loop"}},
    {"id":"writer","parentId":"loop","data":{"kind":"agent","containerId":"loop","agentConfig":{
        "executor":{"agentCli":"c","modelId":"m"},"prompt":"revise"
    }}},
    {"id":"out","data":{"kind":"output","outputs":[{"name":"draft","variableSelector":["loop","result"]}]}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"start","target":"loop"},
    {"source":"entry","target":"writer"},
    {"source":"a","target":"out"},
    {"source":"loop","target":"out"}
]}"#;

type FixtureEngine = WorkflowRunEngine<SqliteWorkflowRunEngineRepository, SeqGen, ClockAt>;

fn live_nodes(pool: &ora_db::RepositoryPool, run_id: &WorkflowRunId) -> Vec<WorkflowNodeRun> {
    SqliteWorkflowRunRepository::new(pool.clone())
        .list_node_runs(run_id)
        .unwrap()
}

fn find_run(pool: &ora_db::RepositoryPool, run_id: &WorkflowRunId) -> ora_domain::WorkflowRun {
    SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(run_id)
        .unwrap()
        .unwrap()
}

fn single<'a>(nodes: &'a [WorkflowNodeRun], node_id: &str) -> &'a WorkflowNodeRun {
    let matching: Vec<_> = nodes
        .iter()
        .filter(|node| node.node_id == node_id)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one live row for {node_id}, got {matching:?}"
    );
    matching[0]
}

fn running_writer(nodes: &[WorkflowNodeRun]) -> &WorkflowNodeRun {
    nodes
        .iter()
        .find(|node| node.node_id == "writer" && node.status == WorkflowNodeStatus::Running)
        .expect("a Running writer row")
}

fn complete(
    engine: &FixtureEngine,
    run_id: &WorkflowRunId,
    node_run_id: &WorkflowNodeRunId,
    output: &str,
) {
    engine
        .complete_node(
            run_id,
            node_run_id,
            Some(output.to_string()),
            None,
            None,
            Vec::new(),
        )
        .unwrap();
}

fn dispatch_counts(executor: &RecordingExecutor) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for record in executor.records() {
        *counts.entry(record.node_id).or_insert(0) += 1;
    }
    counts
}

fn connection(temp: &TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(temp.path().join("repository.sqlite3")).unwrap()
}

/// `(round_index, status)` of every round scope owned by the given Loop row, oldest first.
fn rounds_of(temp: &TempDir, loop_run_id: &WorkflowNodeRunId) -> Vec<(u32, WorkflowScopeStatus)> {
    let connection = connection(temp);
    let mut statement = connection
        .prepare(
            "SELECT round_index, status FROM workflow_execution_scopes
             WHERE parent_loop_node_run_id = ?1 ORDER BY round_index",
        )
        .unwrap();
    statement
        .query_map(params![loop_run_id.as_ref()], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                WorkflowScopeStatus::from_database_value(row.get::<_, i64>(1)?).unwrap(),
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn deleted_rows(temp: &TempDir, run_id: &WorkflowRunId, node_id: &str) -> i64 {
    connection(temp)
        .query_row(
            "SELECT COUNT(*) FROM workflow_node_runs
             WHERE run_id = ?1 AND node_id = ?2 AND is_deleted = 1",
            params![run_id.as_ref(), node_id],
            |row| row.get(0),
        )
        .unwrap()
}

fn active_round_scopes(temp: &TempDir, run_id: &WorkflowRunId) -> i64 {
    connection(temp)
        .query_row(
            "SELECT COUNT(*) FROM workflow_execution_scopes
             WHERE run_id = ?1 AND parent_loop_node_run_id IS NOT NULL AND status IN (0, 1)",
            params![run_id.as_ref()],
            |row| row.get(0),
        )
        .unwrap()
}

/// A failure in round 2 fails the Loop and the run; resume soft-deletes every round row and
/// restarts the Loop from round 1 while the finished rounds keep their history.
#[test]
fn resume_after_a_second_round_failure_restarts_the_loop_from_round_one() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, LOOP_GRAPH, executor.clone());

        let nodes = live_nodes(&pool, &run_id);
        let loop_old = single(&nodes, "loop").id.clone();
        let writer_round_one = running_writer(&nodes).clone();
        complete(&engine, &run_id, &writer_round_one.id, "again");

        let nodes = live_nodes(&pool, &run_id);
        let writer_round_two = running_writer(&nodes).clone();
        assert_ne!(writer_round_two.scope_id, writer_round_one.scope_id);
        engine
            .fail_node(
                &run_id,
                &writer_round_two.id,
                NodeFailure::new(NodeFailureKind::Session, "round two failed"),
            )
            .unwrap();

        let nodes = live_nodes(&pool, &run_id);
        assert_eq!(single(&nodes, "loop").status, WorkflowNodeStatus::Failed);
        assert_eq!(find_run(&pool, &run_id).status, WorkflowRunStatus::Failed);
        assert_eq!(
            rounds_of(&temp, &loop_old),
            vec![
                (1, WorkflowScopeStatus::Succeeded),
                (2, WorkflowScopeStatus::Failed)
            ]
        );

        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );

        let nodes = live_nodes(&pool, &run_id);
        let loop_new = single(&nodes, "loop");
        assert_ne!(loop_new.id, loop_old);
        assert_eq!(loop_new.status, WorkflowNodeStatus::Running);
        let writer_new = single(&nodes, "writer");
        assert_eq!(writer_new.status, WorkflowNodeStatus::Running);
        assert!(
            writer_new.scope_id != writer_round_one.scope_id
                && writer_new.scope_id != writer_round_two.scope_id,
            "the rerun must open a fresh round scope, got {:?}",
            writer_new.scope_id
        );
        assert_eq!(
            rounds_of(&temp, &loop_new.id),
            vec![(1, WorkflowScopeStatus::Running)]
        );
        assert_eq!(active_round_scopes(&temp, &run_id), 1);
        // The old rounds are history, not live state: their rows are gone from the live view
        // and their terminal scope statuses stay untouched.
        assert_eq!(deleted_rows(&temp, &run_id, "writer"), 2);
        assert_eq!(deleted_rows(&temp, &run_id, "entry"), 2);
        assert_eq!(deleted_rows(&temp, &run_id, "loop"), 1);
        assert_eq!(
            rounds_of(&temp, &loop_old),
            vec![
                (1, WorkflowScopeStatus::Succeeded),
                (2, WorkflowScopeStatus::Failed)
            ]
        );
        assert_eq!(dispatch_counts(&executor).get("writer"), Some(&3));

        complete(&engine, &run_id, &writer_new.id, "done");
        let run = find_run(&pool, &run_id);
        assert_eq!(
            (run.status, run.output),
            (
                WorkflowRunStatus::Succeeded,
                Some(r#"{"draft":"done"}"#.to_string())
            )
        );
    });
}

/// Cancelling mid-round closes the round scope; resume opens a fresh round 1 and leaves no
/// second active round behind for the scheduler to pick up.
#[test]
fn cancel_during_a_round_closes_the_scope_and_resume_opens_a_fresh_round_one() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, LOOP_GRAPH, executor.clone());
        let nodes = live_nodes(&pool, &run_id);
        let loop_old = single(&nodes, "loop").id.clone();
        let writer_old = running_writer(&nodes).id.clone();

        assert_eq!(
            engine.cancel(&run_id).unwrap(),
            CancelWorkflowRunResult::Cancelled
        );
        let nodes = live_nodes(&pool, &run_id);
        assert_eq!(single(&nodes, "loop").status, WorkflowNodeStatus::Cancelled);
        assert_eq!(
            single(&nodes, "writer").status,
            WorkflowNodeStatus::Cancelled
        );
        assert_eq!(
            rounds_of(&temp, &loop_old),
            vec![(1, WorkflowScopeStatus::Cancelled)]
        );
        assert_eq!(active_round_scopes(&temp, &run_id), 0);

        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let nodes = live_nodes(&pool, &run_id);
        let loop_new = single(&nodes, "loop");
        assert_ne!(loop_new.id, loop_old);
        assert_eq!(loop_new.status, WorkflowNodeStatus::Running);
        let writer_new = single(&nodes, "writer");
        assert_ne!(writer_new.id, writer_old);
        assert_eq!(writer_new.status, WorkflowNodeStatus::Running);
        assert_eq!(
            rounds_of(&temp, &loop_new.id),
            vec![(1, WorkflowScopeStatus::Running)]
        );
        assert_eq!(active_round_scopes(&temp, &run_id), 1);
        assert_eq!(deleted_rows(&temp, &run_id, "writer"), 1);
        assert_eq!(dispatch_counts(&executor).get("writer"), Some(&2));

        complete(&engine, &run_id, &writer_new.id, "done");
        assert_eq!(
            find_run(&pool, &run_id).status,
            WorkflowRunStatus::Succeeded
        );
    });
}

/// The checkpoint recorded on the Loop row before its rounds ran survives the Loop's own
/// completion: completion merges onto the existing payload instead of replacing it.
#[test]
fn loop_completion_keeps_the_checkpoint_keys_on_the_loop_row() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let (run_id, _, engine) =
            started_run_with(&temp, &pool, LOOP_GRAPH, RecordingExecutor::default());
        let nodes = live_nodes(&pool, &run_id);
        let loop_id = single(&nodes, "loop").id.clone();
        engine
            .record_node_checkpoint(&loop_id, "snapshot-1", Some("abc123"), None)
            .unwrap();

        complete(&engine, &run_id, &running_writer(&nodes).id, "done");

        let nodes = live_nodes(&pool, &run_id);
        let loop_row = single(&nodes, "loop");
        assert_eq!(loop_row.status, WorkflowNodeStatus::Succeeded);
        assert_eq!(loop_row.output, Some(r#"{"result":"done"}"#.to_string()));
        let payload: serde_json::Value =
            serde_json::from_str(loop_row.payload.as_deref().expect("loop payload")).unwrap();
        assert_eq!(payload["checkpoint"], serde_json::json!("abc123"));
        assert_eq!(payload["snapshot_id"], serde_json::json!("snapshot-1"));
        assert_eq!(payload["stop_reason"], serde_json::json!("loop_succeeded"));
        assert_eq!(
            find_run(&pool, &run_id).status,
            WorkflowRunStatus::Succeeded
        );
        drop(temp);
    });
}

/// D2 at the root scope: a failed ordinary sibling does not touch the Loop or its round; the
/// in-flight writer finishes on its own merits, and the run stays resumable afterwards. The
/// resumed run re-dispatches only the failed sibling; the Loop settles its drained round and
/// never re-runs the writer.
#[test]
fn a_root_sibling_failure_leaves_the_loop_round_alone_and_the_run_resumable() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) =
            started_run_with(&temp, &pool, LOOP_WITH_SIBLING_GRAPH, executor.clone());
        let nodes = live_nodes(&pool, &run_id);
        let a_old = single(&nodes, "a").id.clone();
        let loop_id = single(&nodes, "loop").id.clone();
        let writer = running_writer(&nodes).id.clone();

        engine
            .fail_node(
                &run_id,
                &a_old,
                NodeFailure::new(NodeFailureKind::Session, "a failed"),
            )
            .unwrap();
        let nodes = live_nodes(&pool, &run_id);
        assert_eq!(find_run(&pool, &run_id).status, WorkflowRunStatus::Failed);
        assert_eq!(single(&nodes, "loop").status, WorkflowNodeStatus::Running);
        assert_eq!(single(&nodes, "writer").status, WorkflowNodeStatus::Running);
        assert_eq!(
            rounds_of(&temp, &loop_id),
            vec![(1, WorkflowScopeStatus::Running)]
        );

        complete(&engine, &run_id, &writer, "done");
        let nodes = live_nodes(&pool, &run_id);
        assert_eq!(
            single(&nodes, "writer").status,
            WorkflowNodeStatus::Succeeded
        );
        let run = find_run(&pool, &run_id);
        assert_eq!(run.status, WorkflowRunStatus::Failed);
        assert_eq!(run.error.as_deref(), Some("a failed"));

        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let nodes = live_nodes(&pool, &run_id);
        let a_new = single(&nodes, "a");
        assert_ne!(a_new.id, a_old);
        assert_eq!(a_new.status, WorkflowNodeStatus::Running);
        let loop_row = single(&nodes, "loop");
        assert_eq!(loop_row.id, loop_id);
        assert_eq!(loop_row.status, WorkflowNodeStatus::Succeeded);
        assert_eq!(
            single(&nodes, "writer").status,
            WorkflowNodeStatus::Succeeded
        );
        assert_eq!(deleted_rows(&temp, &run_id, "writer"), 0);
        assert_eq!(
            rounds_of(&temp, &loop_id),
            vec![(1, WorkflowScopeStatus::Succeeded)]
        );

        complete(&engine, &run_id, &a_new.id, "ok");
        let run = find_run(&pool, &run_id);
        assert_eq!(
            (run.status, run.output),
            (
                WorkflowRunStatus::Succeeded,
                Some(r#"{"draft":"done"}"#.to_string())
            )
        );
        let counts = dispatch_counts(&executor);
        assert_eq!(counts.get("a"), Some(&2));
        assert_eq!(counts.get("writer"), Some(&1));
    });
}
