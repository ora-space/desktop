//! Agents in these graphs disable automatic retry (`agentConfig.retry`), so a failed attempt
//! fails its node at once as these tests expect; retry behaviour is covered by `retry_tests`.

use super::checkpoint::record_pre_node_checkpoint;
use super::rollback::{apply_rollback, fill_snapshot_preview, plan_rollback, preview};
use super::snapshot_switch::{check_switch, load_context, switch_if_requested};
use super::test_fixture::{
    ClockAt, RecordingExecutor, SeqGen, bootstrap, init_git_workspace, started_run_with,
};
use crate::error::BackendError;
use ora_application::{
    NodeFailure, NodeFailureKind, ResumeWorkflowRunResult, WorkflowGraph, WorkflowRunEngine,
    WorkflowRunPayload, WorkflowRunRepository, WorkflowVariablePool,
};
use ora_application::{PublishSnapshotResult, UpdateDraftResult, WorkflowRepository};
use ora_contracts::{
    PublicError, ResumeRollbackMode, WorkflowRunLocale,
    WorkflowSnapshotIncompatibleWithResumeParams,
};
use ora_db::{
    SqliteWorkflowRepository, SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository,
};
use ora_domain::{
    WorkflowId, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId,
    WorkflowRunStatus, WorkflowSnapshotId,
};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const V1_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"b","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"old-c",
        "outputContract":{"type":"structured","textExposure":"includeFinalText",
            "schema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}}}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"a","target":"b"},
    {"source":"b","target":"c"},
    {"source":"c","target":"output"}
]}"#;

const V2_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"b","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"fixed-c",
        "outputContract":{"type":"structured","textExposure":"includeFinalText",
            "schema":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}}}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"a","target":"b"},
    {"source":"b","target":"c"},
    {"source":"c","target":"output"}
]}"#;

const V3_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"c"}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"a","target":"c"},
    {"source":"c","target":"output"}
]}"#;

type FixtureEngine = WorkflowRunEngine<SqliteWorkflowRunEngineRepository, SeqGen, ClockAt>;

struct FailedCFixture {
    temp: TempDir,
    pool: ora_db::RepositoryPool,
    run_id: WorkflowRunId,
    engine: FixtureEngine,
    executor: RecordingExecutor,
    c_old: WorkflowNodeRunId,
}

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

fn complete(engine: &FixtureEngine, run_id: &WorkflowRunId, node_run_id: &WorkflowNodeRunId) {
    engine
        .complete_node(
            run_id,
            node_run_id,
            Some("ok".to_string()),
            /*structured_output*/ None,
            /*stop_reason*/ None,
            Vec::new(),
        )
        .unwrap();
}

fn find_run(pool: &ora_db::RepositoryPool, run_id: &WorkflowRunId) -> ora_domain::WorkflowRun {
    SqliteWorkflowRunRepository::new(pool.clone())
        .find_run(run_id)
        .unwrap()
        .unwrap()
}

fn dispatch_counts(executor: &RecordingExecutor) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for node_id in executor.dispatches.lock().expect("dispatch log").iter() {
        *counts.entry(node_id.clone()).or_insert(0) += 1;
    }
    counts
}

fn seed_payload(temp: &TempDir, run_id: &WorkflowRunId, graph: &str) {
    let parsed = WorkflowGraph::parse(graph).unwrap();
    let mut variable_pool = WorkflowVariablePool::from_graph(&parsed);
    variable_pool
        .values
        .insert("sys.workflow_id".to_string(), json!("workflow-1"));
    let payload = WorkflowRunPayload::with_variable_pool(
        WorkflowRunLocale::ZhCn,
        Default::default(),
        Some("start".to_string()),
        variable_pool,
    );
    let serialized = serde_json::to_string(&payload).unwrap();
    let connection = rusqlite::Connection::open(temp.path().join("repository.sqlite3")).unwrap();
    connection
        .execute(
            "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1",
            rusqlite::params![run_id.as_ref(), serialized],
        )
        .unwrap();
}

fn publish_version(
    pool: &ora_db::RepositoryPool,
    snapshot_id: &str,
    version: &str,
    graph: &str,
    created_at: i64,
) -> WorkflowSnapshotId {
    let workflow_repo = SqliteWorkflowRepository::new(pool.clone());
    let workflow_id = WorkflowId::new("workflow-1");
    match workflow_repo
        .update_draft(&workflow_id, graph.to_string(), created_at)
        .unwrap()
    {
        UpdateDraftResult::Updated(_) => {}
        other => panic!("expected updated draft, got {other:?}"),
    }
    let id = WorkflowSnapshotId::new(snapshot_id);
    match workflow_repo
        .publish_snapshot(&workflow_id, id.clone(), version.to_string(), created_at)
        .unwrap()
    {
        PublishSnapshotResult::Published(snapshot) => snapshot.id,
        other => panic!("expected published snapshot, got {other:?}"),
    }
}

fn parse_payload(run: &ora_domain::WorkflowRun) -> WorkflowRunPayload {
    serde_json::from_str(run.payload.as_deref().expect("run payload")).unwrap()
}

fn fail_c() -> FailedCFixture {
    let (temp, pool) = bootstrap();
    let executor = RecordingExecutor::default();
    let (run_id, _, engine) = started_run_with(&temp, &pool, V1_GRAPH, executor.clone());
    seed_payload(&temp, &run_id, V1_GRAPH);
    let nodes = live_nodes(&pool, &run_id);
    complete(&engine, &run_id, &live(&nodes, "a").id);
    let after_a = live_nodes(&pool, &run_id);
    complete(&engine, &run_id, &live(&after_a, "b").id);
    let after_b = live_nodes(&pool, &run_id);
    let c_old = live(&after_b, "c").id.clone();
    engine
        .fail_node(
            &run_id,
            &c_old,
            NodeFailure::new(NodeFailureKind::StructuredOutput, "schema mismatch"),
        )
        .unwrap();
    FailedCFixture {
        temp,
        pool,
        run_id,
        engine,
        executor,
        c_old,
    }
}

fn skills_root(temp: &TempDir) -> std::path::PathBuf {
    let root = temp.path().join("skills");
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn workspace_root(temp: &TempDir) -> std::path::PathBuf {
    temp.path().join("fixture-project")
}

fn switch(fixture: &FailedCFixture, snapshot_id: &str) -> Result<(), BackendError> {
    switch_if_requested(
        &fixture.pool,
        &skills_root(&fixture.temp),
        &workspace_root(&fixture.temp),
        &fixture.run_id,
        Some(snapshot_id),
        90,
    )
    .map(|_| ())
}

fn preview_with_snapshots(
    pool: &ora_db::RepositoryPool,
    workspace_root: &Path,
    run_id: &WorkflowRunId,
) -> ora_contracts::PreviewWorkflowRunResumeResponse {
    let mut response = preview(pool, workspace_root, run_id).unwrap();
    fill_snapshot_preview(pool, run_id, &mut response).unwrap();
    response
}

/// (1) Resume onto published v2 re-runs only C and keeps A/B outputs.
#[test]
fn resume_switches_to_published_v2_and_redoes_only_the_failed_node() {
    with_trace_logging(|| {
        let fixture = fail_c();
        let before = find_run(&fixture.pool, &fixture.run_id);
        assert_eq!(before.snapshot_id, WorkflowSnapshotId::new("snapshot-1"));
        let revision_before = parse_payload(&before).variable_pool.revision;
        publish_version(&fixture.pool, "snapshot-2", "v2", V2_GRAPH, 50);
        switch(&fixture, "snapshot-2").unwrap();
        let switched = find_run(&fixture.pool, &fixture.run_id);
        println!(
            "snapshot before={} after={} pool={:?}",
            before.snapshot_id,
            switched.snapshot_id,
            parse_payload(&switched)
                .variable_pool
                .values
                .keys()
                .collect::<Vec<_>>()
        );
        assert_eq!(switched.snapshot_id, WorkflowSnapshotId::new("snapshot-2"));
        let payload = parse_payload(&switched);
        assert_eq!(payload.start_node_id.as_deref(), Some("start"));
        assert_eq!(payload.variable_pool.revision, revision_before + 1);
        assert!(payload.variable_pool.values.contains_key("a.output"));
        assert!(payload.variable_pool.values.contains_key("b.output"));

        assert_eq!(
            fixture.engine.resume_from_failure(&fixture.run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let counts = dispatch_counts(&fixture.executor);
        println!("dispatch counts: {counts:?}");
        assert_eq!(counts.get("a"), Some(&1));
        assert_eq!(counts.get("b"), Some(&1));
        assert_eq!(counts.get("c"), Some(&2));
        assert_eq!(
            find_run(&fixture.pool, &fixture.run_id).status,
            WorkflowRunStatus::Running
        );
        let after = live_nodes(&fixture.pool, &fixture.run_id);
        let c_new = live(&after, "c");
        assert_ne!(c_new.id, fixture.c_old);
        complete(&fixture.engine, &fixture.run_id, &c_new.id);
        assert_eq!(
            find_run(&fixture.pool, &fixture.run_id).status,
            WorkflowRunStatus::Succeeded
        );
    });
}

/// (2) A published snapshot that dropped a succeeded node is refused and leaves the run untouched.
#[test]
fn resume_rejects_a_snapshot_that_removed_a_succeeded_node() {
    with_trace_logging(|| {
        let fixture = fail_c();
        publish_version(&fixture.pool, "snapshot-3", "v3", V3_GRAPH, 60);
        let error = switch(&fixture, "snapshot-3").expect_err("v3 must be incompatible");
        let public = error.public_error().clone();
        println!("{}", serde_json::to_string(&public).unwrap());
        assert_eq!(
            serde_json::to_value(&public).unwrap(),
            json!({
                "code": "workflow_snapshot_incompatible_with_resume",
                "params": { "reason": "node_missing:b" },
            })
        );
        assert_eq!(
            public,
            PublicError::WorkflowSnapshotIncompatibleWithResume(
                WorkflowSnapshotIncompatibleWithResumeParams {
                    reason: "node_missing:b".to_string(),
                }
            )
        );
        let run = find_run(&fixture.pool, &fixture.run_id);
        assert_eq!(run.status, WorkflowRunStatus::Failed);
        assert_eq!(run.snapshot_id, WorkflowSnapshotId::new("snapshot-1"));
        let nodes = live_nodes(&fixture.pool, &fixture.run_id);
        assert_eq!(live(&nodes, "a").status, WorkflowNodeStatus::Succeeded);
        assert_eq!(live(&nodes, "b").status, WorkflowNodeStatus::Succeeded);
        assert_eq!(live(&nodes, "c").status, WorkflowNodeStatus::Failed);
        assert_eq!(live(&nodes, "c").id, fixture.c_old);
    });
}

/// (3) Naming the current snapshot is a keep-resume no-op for the snapshot pointer.
#[test]
fn resume_with_the_current_snapshot_id_keeps_the_run_on_v1() {
    with_trace_logging(|| {
        let fixture = fail_c();
        switch(&fixture, "snapshot-1").unwrap();
        assert_eq!(
            find_run(&fixture.pool, &fixture.run_id).snapshot_id,
            WorkflowSnapshotId::new("snapshot-1")
        );
        assert_eq!(
            fixture.engine.resume_from_failure(&fixture.run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let counts = dispatch_counts(&fixture.executor);
        assert_eq!(counts.get("a"), Some(&1));
        assert_eq!(counts.get("b"), Some(&1));
        assert_eq!(counts.get("c"), Some(&2));
        assert_eq!(
            find_run(&fixture.pool, &fixture.run_id).snapshot_id,
            WorkflowSnapshotId::new("snapshot-1")
        );
    });
}

/// (4) Preview reports whether the currently published snapshot can take over this run.
#[test]
fn preview_reports_published_snapshot_switchability() {
    with_trace_logging(|| {
        let fixture = fail_c();
        let workspace = init_git_workspace(&fixture.temp);
        publish_version(&fixture.pool, "snapshot-2", "v2", V2_GRAPH, 50);
        let v2 = preview_with_snapshots(&fixture.pool, &workspace, &fixture.run_id);
        assert_eq!(v2.current_snapshot_id, "snapshot-1");
        assert_eq!(v2.current_snapshot_version, "v1");
        assert_eq!(v2.published_snapshot_id.as_deref(), Some("snapshot-2"));
        assert_eq!(v2.published_snapshot_version.as_deref(), Some("v2"));
        assert!(v2.published_snapshot_switchable);
        assert_eq!(v2.published_snapshot_incompatible_reason, None);

        publish_version(&fixture.pool, "snapshot-3", "v3", V3_GRAPH, 60);
        let v3 = preview_with_snapshots(&fixture.pool, &workspace, &fixture.run_id);
        assert_eq!(v3.published_snapshot_id.as_deref(), Some("snapshot-3"));
        assert!(!v3.published_snapshot_switchable);
        assert_eq!(
            v3.published_snapshot_incompatible_reason.as_deref(),
            Some("node_missing:b")
        );
    });
}

/// (5) The executor checkpoint path writes the run snapshot id onto the node payload.
#[test]
fn record_pre_node_checkpoint_writes_payload_snapshot_id() {
    with_trace_logging(|| {
        let fixture = fail_c();
        let workspace = init_git_workspace(&fixture.temp);
        let context = load_context(&fixture.pool, &fixture.run_id).unwrap();
        let repository = SqliteWorkflowRunEngineRepository::new(fixture.pool.clone());
        record_pre_node_checkpoint(
            &repository,
            &workspace,
            &fixture.run_id,
            "c",
            &fixture.c_old,
            context.run.snapshot_id.as_ref(),
            95,
        )
        .unwrap();
        let nodes = live_nodes(&fixture.pool, &fixture.run_id);
        let payload: serde_json::Value =
            serde_json::from_str(live(&nodes, "c").payload.as_deref().unwrap()).unwrap();
        assert_eq!(payload["snapshot_id"], "snapshot-1");
    });
}

fn merge_payload_snapshot_id(temp: &TempDir, node_run_id: &str, snapshot_id: &str) {
    let connection = rusqlite::Connection::open(temp.path().join("repository.sqlite3")).unwrap();
    let current: Option<String> = connection
        .query_row(
            "SELECT payload FROM workflow_node_runs WHERE id = ?1",
            rusqlite::params![node_run_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut payload: serde_json::Value = current
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| json!({}));
    payload["snapshot_id"] = json!(snapshot_id);
    connection
        .execute(
            "UPDATE workflow_node_runs SET payload = ?2 WHERE id = ?1",
            rusqlite::params![node_run_id, payload.to_string()],
        )
        .unwrap();
}

fn git_pre_rollback_refs(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            "refs/ora/checkpoints/",
        ])
        .output()
        .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| line.contains("pre-rollback-"))
        .map(str::to_string)
        .collect()
}

/// B9: an incompatible snapshot is refused before any worktree mutation.
#[test]
fn incompatible_snapshot_switch_does_not_touch_rows_files_or_pre_rollback_ref() {
    with_trace_logging(|| {
        let fixture = fail_c();
        let workspace = init_git_workspace(&fixture.temp);
        std::fs::write(workspace.join("kept.txt"), "before\n").unwrap();
        let repository = SqliteWorkflowRunEngineRepository::new(fixture.pool.clone());
        record_pre_node_checkpoint(
            &repository,
            &workspace,
            &fixture.run_id,
            "c",
            &fixture.c_old,
            "snapshot-1",
            95,
        )
        .unwrap();
        std::fs::write(workspace.join("kept.txt"), "after\n").unwrap();
        std::fs::write(workspace.join("extra.txt"), "new\n").unwrap();
        publish_version(&fixture.pool, "snapshot-3", "v3", V3_GRAPH, 60);
        let rows_before = live_nodes(&fixture.pool, &fixture.run_id);
        let run_before = find_run(&fixture.pool, &fixture.run_id);
        let context = load_context(&fixture.pool, &fixture.run_id).unwrap();
        let target = SqliteWorkflowRepository::new(fixture.pool.clone())
            .find_snapshot_by_id(&context.workflow.id, &WorkflowSnapshotId::new("snapshot-3"))
            .unwrap()
            .unwrap();
        let error = check_switch(&fixture.pool, &context, &target).expect_err("v3 must be refused");
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowSnapshotIncompatibleWithResume(
                WorkflowSnapshotIncompatibleWithResumeParams {
                    reason: "node_missing:b".to_string(),
                }
            )
        );
        assert_eq!(find_run(&fixture.pool, &fixture.run_id), run_before);
        assert_eq!(live_nodes(&fixture.pool, &fixture.run_id), rows_before);
        assert_eq!(
            std::fs::read_to_string(workspace.join("kept.txt")).unwrap(),
            "after\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("extra.txt")).unwrap(),
            "new\n"
        );
        assert_eq!(git_pre_rollback_refs(&workspace), Vec::<String>::new());
        assert_eq!(run_before.status, WorkflowRunStatus::Failed);
    });
}

/// B9: a compatible published snapshot plus checkpoint rollback both apply, and only rerun rows
/// pick up the new snapshot id.
#[test]
fn compatible_snapshot_switch_with_checkpoint_records_new_id_only_on_rerun_rows() {
    with_trace_logging(|| {
        let fixture = fail_c();
        let workspace = init_git_workspace(&fixture.temp);
        let nodes = live_nodes(&fixture.pool, &fixture.run_id);
        merge_payload_snapshot_id(&fixture.temp, live(&nodes, "a").id.as_ref(), "snapshot-1");
        merge_payload_snapshot_id(&fixture.temp, live(&nodes, "b").id.as_ref(), "snapshot-1");
        let repository = SqliteWorkflowRunEngineRepository::new(fixture.pool.clone());
        record_pre_node_checkpoint(
            &repository,
            &workspace,
            &fixture.run_id,
            "c",
            &fixture.c_old,
            "snapshot-1",
            95,
        )
        .unwrap();
        std::fs::write(workspace.join("from-c.txt"), "dirty\n").unwrap();
        publish_version(&fixture.pool, "snapshot-2", "v2", V2_GRAPH, 50);
        let plan = plan_rollback(&fixture.pool, &fixture.run_id).unwrap();
        apply_rollback(
            &workspace,
            &plan,
            ResumeRollbackMode::Checkpoint,
            &fixture.run_id,
            99,
        )
        .unwrap();
        assert!(!workspace.join("from-c.txt").exists());
        switch(&fixture, "snapshot-2").unwrap();
        assert_eq!(
            fixture.engine.resume_from_failure(&fixture.run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let after = live_nodes(&fixture.pool, &fixture.run_id);
        let a = live(&after, "a");
        let b = live(&after, "b");
        let c_new = live(&after, "c");
        assert_ne!(c_new.id, fixture.c_old);
        let context = load_context(&fixture.pool, &fixture.run_id).unwrap();
        record_pre_node_checkpoint(
            &repository,
            &workspace,
            &fixture.run_id,
            "c",
            &c_new.id,
            context.run.snapshot_id.as_ref(),
            110,
        )
        .unwrap();
        let a_payload: serde_json::Value =
            serde_json::from_str(a.payload.as_deref().unwrap()).unwrap();
        let b_payload: serde_json::Value =
            serde_json::from_str(b.payload.as_deref().unwrap()).unwrap();
        let c_payload: serde_json::Value = serde_json::from_str(
            live(&live_nodes(&fixture.pool, &fixture.run_id), "c")
                .payload
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(a_payload["snapshot_id"], "snapshot-1");
        assert_eq!(b_payload["snapshot_id"], "snapshot-1");
        assert_eq!(c_payload["snapshot_id"], "snapshot-2");
        assert_eq!(
            find_run(&fixture.pool, &fixture.run_id).snapshot_id,
            WorkflowSnapshotId::new("snapshot-2")
        );
        assert!(!git_pre_rollback_refs(&workspace).is_empty());
    });
}
