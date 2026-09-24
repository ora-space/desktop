//! Agents in these graphs disable automatic retry (`agentConfig.retry`), so a failed attempt
//! fails its node at once as these tests expect; retry behaviour is covered by `retry_tests`.

use super::checkpoint::take_checkpoint;
use super::rollback::{apply_rollback, plan_rollback, preview};
use super::test_fixture::{
    ClockAt, RecordingExecutor, SeqGen, TWO_AGENT_GRAPH, bootstrap, init_git_workspace,
    started_run_with,
};
use crate::error::BackendError;
use ora_application::{
    FileChange, NodeFailure, NodeFailureKind, ResumeWorkflowRunResult, WorkflowRunEngine,
    WorkflowRunRepository,
};
use ora_contracts::{
    EmptyErrorParams, PreviewWorkflowRunResumeResponse, PublicError, ResumeRollbackMode,
};
use ora_db::{SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunId, WorkflowRunStatus};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use rusqlite::params;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

const U5_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"c"}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"a","target":"c"},
    {"source":"c","target":"output"}
]}"#;

const SIBLING_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"b","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"c","modelId":"m"},"prompt":"c"}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"start","target":"b"},
    {"source":"a","target":"c"},
    {"source":"c","target":"output"}
]}"#;

const PRE_ROLLBACK_NOW: i64 = 99;

type FixtureEngine = WorkflowRunEngine<SqliteWorkflowRunEngineRepository, SeqGen, ClockAt>;

struct FailedCFixture {
    _temp: TempDir,
    pool: ora_db::RepositoryPool,
    run_id: WorkflowRunId,
    workspace_root: PathBuf,
    engine: FixtureEngine,
    executor: RecordingExecutor,
    c_old: ora_domain::WorkflowNodeRunId,
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

fn write_workspace_file(root: &Path, name: &str, contents: &str) {
    std::fs::write(root.join(name), contents).unwrap();
}

fn checkpoint_refs(root: &Path) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(["for-each-ref", "refs/ora/checkpoints/"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git for-each-ref failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn pre_rollback_refs(root: &Path) -> Vec<String> {
    checkpoint_refs(root)
        .lines()
        .filter(|line| line.contains("pre-rollback-"))
        .map(str::to_string)
        .collect()
}

fn set_started_at(temp: &TempDir, node_run_id: &str, started_at: i64) {
    set_node_times(temp, node_run_id, Some(started_at), None);
}

fn set_finished_at(temp: &TempDir, node_run_id: &str, finished_at: i64) {
    set_node_times(temp, node_run_id, None, Some(finished_at));
}

fn set_node_times(
    temp: &TempDir,
    node_run_id: &str,
    started_at: Option<i64>,
    finished_at: Option<i64>,
) {
    let connection = rusqlite::Connection::open(temp.path().join("repository.sqlite3")).unwrap();
    connection.busy_timeout(Duration::from_secs(5)).unwrap();
    if let Some(started_at) = started_at {
        connection
            .execute(
                "UPDATE workflow_node_runs SET started_at = ?2 WHERE id = ?1",
                params![node_run_id, started_at],
            )
            .unwrap();
    }
    if let Some(finished_at) = finished_at {
        connection
            .execute(
                "UPDATE workflow_node_runs SET finished_at = ?2 WHERE id = ?1",
                params![node_run_id, finished_at],
            )
            .unwrap();
    }
}

fn recorded_changes() -> Vec<FileChange> {
    vec![
        FileChange {
            path: "f1".to_string(),
            additions: 1,
            deletions: 0,
        },
        FileChange {
            path: "f2".to_string(),
            additions: 1,
            deletions: 0,
        },
    ]
}

fn fail_c_after_checkpoint(graph: &str) -> FailedCFixture {
    let (temp, pool) = bootstrap();
    let executor = RecordingExecutor::default();
    let (run_id, _, engine) = started_run_with(&temp, &pool, graph, executor.clone());
    let workspace_root = init_git_workspace(&temp);
    let nodes = live_nodes(&pool, &run_id);
    complete(&engine, &run_id, &live(&nodes, "a").id);
    let after_a = live_nodes(&pool, &run_id);
    let c_old = live(&after_a, "c").id.clone();
    let checkpoint = take_checkpoint(&workspace_root, &run_id, "c", &c_old);
    let oid = checkpoint.commit_oid.expect("checkpoint oid");
    engine
        .record_node_checkpoint(
            &c_old,
            "snapshot-1",
            Some(&oid),
            /*checkpoint_error*/ None,
        )
        .unwrap();
    write_workspace_file(&workspace_root, "f1", "node-f1\n");
    write_workspace_file(&workspace_root, "f2", "node-f2\n");
    engine
        .fail_node(
            &run_id,
            &c_old,
            NodeFailure::new(NodeFailureKind::Session, "c failed")
                .with_file_changes(recorded_changes()),
        )
        .unwrap();
    write_workspace_file(&workspace_root, "f3", "manual\n");
    FailedCFixture {
        _temp: temp,
        pool,
        run_id,
        workspace_root,
        engine,
        executor,
        c_old,
    }
}

fn resume_from_failure(
    fixture: &FailedCFixture,
    mode: ResumeRollbackMode,
) -> Result<(ResumeWorkflowRunResult, Option<String>), BackendError> {
    let pre_rollback_checkpoint = if mode == ResumeRollbackMode::Keep {
        None
    } else {
        let plan = plan_rollback(&fixture.pool, &fixture.run_id)?;
        if !plan.resumable() {
            return Err(BackendError::from(
                ora_application::ApplicationError::WorkflowRunNotResumable,
            ));
        }
        apply_rollback(
            &fixture.workspace_root,
            &plan,
            mode,
            &fixture.run_id,
            PRE_ROLLBACK_NOW,
        )?
    };
    let result = fixture.engine.resume_from_failure(&fixture.run_id).unwrap();
    Ok((result, pre_rollback_checkpoint))
}

fn preview_paths(preview: &PreviewWorkflowRunResumeResponse) -> (Vec<String>, Vec<String>) {
    let node = &preview.failed_nodes[0];
    let recorded: Vec<String> = node
        .node_file_changes
        .iter()
        .map(|change| change.path.clone())
        .collect();
    let mut since: Vec<String> = node
        .changed_since_checkpoint
        .iter()
        .map(|change| change.path.clone())
        .collect();
    since.sort();
    (recorded, since)
}

/// (a) Preview reports the failed node's recorded files and the live worktree delta.
#[test]
fn preview_reports_node_files_and_live_changes() {
    with_trace_logging(|| {
        let fixture = fail_c_after_checkpoint(U5_GRAPH);
        let response = preview(&fixture.pool, &fixture.workspace_root, &fixture.run_id).unwrap();
        println!(
            "{}",
            serde_json::to_string_pretty(&response).expect("preview json")
        );
        assert!(response.resumable);
        assert_eq!(response.failed_nodes.len(), 1);
        assert_eq!(response.failed_nodes[0].node_id, "c");
        let (recorded, since) = preview_paths(&response);
        assert_eq!(recorded, vec!["f1".to_string(), "f2".to_string()]);
        assert_eq!(
            since,
            vec!["f1".to_string(), "f2".to_string(), "f3".to_string()]
        );
        assert!(response.node_files_available);
        assert!(response.checkpoint_available);
    });
}

/// (b) NodeFiles restores the failed node's paths, keeps the manual edit, and re-dispatches C.
#[test]
fn resume_node_files_restores_recorded_paths_and_keeps_manual_edits() {
    with_trace_logging(|| {
        let fixture = fail_c_after_checkpoint(U5_GRAPH);
        let (result, pre_rollback) =
            resume_from_failure(&fixture, ResumeRollbackMode::NodeFiles).unwrap();
        assert_eq!(result, ResumeWorkflowRunResult::Resumed);
        assert!(pre_rollback.is_some());
        assert!(!fixture.workspace_root.join("f1").exists());
        assert!(!fixture.workspace_root.join("f2").exists());
        assert_eq!(
            std::fs::read_to_string(fixture.workspace_root.join("f3")).unwrap(),
            "manual\n"
        );
        let refs = checkpoint_refs(&fixture.workspace_root);
        println!("{refs}");
        let pre_refs = pre_rollback_refs(&fixture.workspace_root);
        assert_eq!(pre_refs.len(), 1);
        assert!(pre_refs[0].contains(&format!(
            "pre-rollback-{}-{PRE_ROLLBACK_NOW}",
            fixture.run_id
        )));
        let after = live_nodes(&fixture.pool, &fixture.run_id);
        let c_new = live(&after, "c");
        assert_ne!(c_new.id, fixture.c_old);
        assert_eq!(c_new.status, WorkflowNodeStatus::Running);
        assert_eq!(
            find_run(&fixture.pool, &fixture.run_id).status,
            WorkflowRunStatus::Running
        );
        assert_eq!(dispatch_counts(&fixture.executor).get("c"), Some(&2));
    });
}

/// (c) Checkpoint restores every path, including the manual edit made after failure.
#[test]
fn resume_checkpoint_restores_the_whole_worktree() {
    with_trace_logging(|| {
        let fixture = fail_c_after_checkpoint(U5_GRAPH);
        let (result, pre_rollback) =
            resume_from_failure(&fixture, ResumeRollbackMode::Checkpoint).unwrap();
        assert_eq!(result, ResumeWorkflowRunResult::Resumed);
        assert!(pre_rollback.is_some());
        assert!(!fixture.workspace_root.join("f1").exists());
        assert!(!fixture.workspace_root.join("f2").exists());
        assert!(!fixture.workspace_root.join("f3").exists());
        assert_eq!(pre_rollback_refs(&fixture.workspace_root).len(), 1);
    });
}

/// (d) A later sibling success blocks Checkpoint rollback and leaves the worktree untouched.
#[test]
fn resume_checkpoint_rejects_when_siblings_ran_after() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, SIBLING_GRAPH, executor);
        let workspace_root = init_git_workspace(&temp);
        let nodes = live_nodes(&pool, &run_id);
        let b_id = live(&nodes, "b").id.clone();
        complete(&engine, &run_id, &live(&nodes, "a").id);
        let after_a = live_nodes(&pool, &run_id);
        let c_old = live(&after_a, "c").id.clone();
        let checkpoint = take_checkpoint(&workspace_root, &run_id, "c", &c_old);
        let oid = checkpoint.commit_oid.expect("checkpoint oid");
        engine
            .record_node_checkpoint(
                &c_old,
                "snapshot-1",
                Some(&oid),
                /*checkpoint_error*/ None,
            )
            .unwrap();
        write_workspace_file(&workspace_root, "f1", "node-f1\n");
        write_workspace_file(&workspace_root, "f2", "node-f2\n");
        complete(&engine, &run_id, &b_id);
        set_started_at(&temp, b_id.as_ref(), 50);
        engine
            .fail_node(
                &run_id,
                &c_old,
                NodeFailure::new(NodeFailureKind::Session, "c failed")
                    .with_file_changes(recorded_changes()),
            )
            .unwrap();
        write_workspace_file(&workspace_root, "f3", "manual\n");
        let response = preview(&pool, &workspace_root, &run_id).unwrap();
        assert!(!response.checkpoint_available);
        assert_eq!(
            response.checkpoint_unavailable_reason.as_deref(),
            Some("siblings_ran_after_checkpoint")
        );
        let plan = plan_rollback(&pool, &run_id).unwrap();
        let error = apply_rollback(
            &workspace_root,
            &plan,
            ResumeRollbackMode::Checkpoint,
            &run_id,
            PRE_ROLLBACK_NOW,
        )
        .expect_err("checkpoint rollback must be rejected");
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowRunNotResumable(EmptyErrorParams {})
        );
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("f1")).unwrap(),
            "node-f1\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("f2")).unwrap(),
            "node-f2\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("f3")).unwrap(),
            "manual\n"
        );
        assert_eq!(pre_rollback_refs(&workspace_root), Vec::<String>::new());
    });
}

/// (e) Keep (and an omitted rollback, which maps to Keep) leave files and git refs untouched.
#[test]
fn resume_keep_and_omitted_rollback_leave_the_worktree_untouched() {
    with_trace_logging(|| {
        let fixture = fail_c_after_checkpoint(U5_GRAPH);
        let before_refs = checkpoint_refs(&fixture.workspace_root);
        let omitted: Option<ResumeRollbackMode> = None;
        let mode = omitted.unwrap_or(ResumeRollbackMode::Keep);
        let (result, pre_rollback) = resume_from_failure(&fixture, mode).unwrap();
        assert_eq!(result, ResumeWorkflowRunResult::Resumed);
        assert_eq!(pre_rollback, None);
        assert_eq!(
            std::fs::read_to_string(fixture.workspace_root.join("f1")).unwrap(),
            "node-f1\n"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.workspace_root.join("f3")).unwrap(),
            "manual\n"
        );
        assert_eq!(checkpoint_refs(&fixture.workspace_root), before_refs);
        assert_eq!(
            find_run(&fixture.pool, &fixture.run_id).status,
            WorkflowRunStatus::Running
        );
        let after = live_nodes(&fixture.pool, &fixture.run_id);
        assert_eq!(live(&after, "c").status, WorkflowNodeStatus::Running);
        assert_eq!(dispatch_counts(&fixture.executor).get("c"), Some(&2));
    });
}

fn checkpoint_and_fail_node(
    engine: &FixtureEngine,
    workspace_root: &Path,
    run_id: &WorkflowRunId,
    node: &ora_domain::WorkflowNodeRun,
    message: &str,
) {
    let checkpoint = take_checkpoint(workspace_root, run_id, &node.node_id, &node.id);
    let oid = checkpoint.commit_oid.expect("checkpoint oid");
    engine
        .record_node_checkpoint(
            &node.id,
            "snapshot-1",
            Some(&oid),
            /*checkpoint_error*/ None,
        )
        .unwrap();
    engine
        .fail_node(
            run_id,
            &node.id,
            NodeFailure::new(NodeFailureKind::Session, message)
                .with_file_changes(recorded_changes()),
        )
        .unwrap();
}

/// Same-wave sibling that keeps running (D2) and finishes after the failed node blocks checkpoint.
#[test]
fn resume_checkpoint_rejects_when_same_wave_sibling_finished_after() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, TWO_AGENT_GRAPH, executor);
        let workspace_root = init_git_workspace(&temp);
        let nodes = live_nodes(&pool, &run_id);
        let l = live(&nodes, "l").clone();
        let r = live(&nodes, "r").clone();
        checkpoint_and_fail_node(&engine, &workspace_root, &run_id, &r, "r failed");
        write_workspace_file(&workspace_root, "f1", "sibling-l\n");
        complete(&engine, &run_id, &l.id);
        set_finished_at(&temp, l.id.as_ref(), 80);
        let response = preview(&pool, &workspace_root, &run_id).unwrap();
        assert!(response.resumable);
        assert!(!response.checkpoint_available);
        assert_eq!(
            response.checkpoint_unavailable_reason.as_deref(),
            Some("siblings_ran_after_checkpoint")
        );
        let plan = plan_rollback(&pool, &run_id).unwrap();
        let error = apply_rollback(
            &workspace_root,
            &plan,
            ResumeRollbackMode::Checkpoint,
            &run_id,
            PRE_ROLLBACK_NOW,
        )
        .expect_err("checkpoint rollback must be rejected");
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowRunNotResumable(EmptyErrorParams {})
        );
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("f1")).unwrap(),
            "sibling-l\n"
        );
        assert_eq!(pre_rollback_refs(&workspace_root), Vec::<String>::new());
    });
}

/// A sibling that already finished before the failed node started does not block checkpoint.
#[test]
fn resume_checkpoint_allows_sibling_that_finished_before_unit_started() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, SIBLING_GRAPH, executor);
        let workspace_root = init_git_workspace(&temp);
        let nodes = live_nodes(&pool, &run_id);
        let a = live(&nodes, "a").clone();
        let b = live(&nodes, "b").clone();
        complete(&engine, &run_id, &b.id);
        set_finished_at(&temp, b.id.as_ref(), 30);
        complete(&engine, &run_id, &a.id);
        let after_a = live_nodes(&pool, &run_id);
        let c = live(&after_a, "c").clone();
        set_started_at(&temp, c.id.as_ref(), 50);
        let checkpoint = take_checkpoint(&workspace_root, &run_id, "c", &c.id);
        let oid = checkpoint.commit_oid.expect("checkpoint oid");
        engine
            .record_node_checkpoint(
                &c.id,
                "snapshot-1",
                Some(&oid),
                /*checkpoint_error*/ None,
            )
            .unwrap();
        write_workspace_file(&workspace_root, "f1", "node-f1\n");
        engine
            .fail_node(
                &run_id,
                &c.id,
                NodeFailure::new(NodeFailureKind::Session, "c failed")
                    .with_file_changes(recorded_changes()),
            )
            .unwrap();
        let response = preview(&pool, &workspace_root, &run_id).unwrap();
        assert!(response.resumable);
        assert!(response.checkpoint_available);
        assert_eq!(response.checkpoint_unavailable_reason, None);
        let plan = plan_rollback(&pool, &run_id).unwrap();
        apply_rollback(
            &workspace_root,
            &plan,
            ResumeRollbackMode::Checkpoint,
            &run_id,
            PRE_ROLLBACK_NOW,
        )
        .expect("checkpoint rollback must apply");
        assert!(!workspace_root.join("f1").exists());
        assert_eq!(pre_rollback_refs(&workspace_root).len(), 1);
    });
}

/// A still-running sibling makes the run itself not resumable; checkpoint is not offered.
#[test]
fn resume_is_refused_while_a_sibling_is_still_running() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, TWO_AGENT_GRAPH, executor.clone());
        let workspace_root = init_git_workspace(&temp);
        let nodes = live_nodes(&pool, &run_id);
        let l = live(&nodes, "l").clone();
        let r = live(&nodes, "r").clone();
        checkpoint_and_fail_node(&engine, &workspace_root, &run_id, &r, "r failed");
        let before = live_nodes(&pool, &run_id);
        let l_before = live(&before, "l").clone();
        let r_before = live(&before, "r").clone();
        let response = preview(&pool, &workspace_root, &run_id).unwrap();
        assert_eq!(response.resumable, false);
        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::NotResumable
        );
        let after_refused = live_nodes(&pool, &run_id);
        assert_eq!(live(&after_refused, "l").id, l_before.id);
        assert_eq!(live(&after_refused, "l").started_at, l_before.started_at);
        assert_eq!(live(&after_refused, "r").id, r_before.id);
        assert_eq!(find_run(&pool, &run_id).status, WorkflowRunStatus::Failed);
        complete(&engine, &run_id, &l.id);
        let after_sibling = live_nodes(&pool, &run_id);
        let l_done = live(&after_sibling, "l").clone();
        let preview_ready = preview(&pool, &workspace_root, &run_id).unwrap();
        assert!(preview_ready.resumable);
        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        let after_resume = live_nodes(&pool, &run_id);
        let l_kept = live(&after_resume, "l");
        assert_eq!(l_kept.id, l_done.id);
        assert_eq!(l_kept.started_at, l_done.started_at);
        assert_eq!(l_kept.status, WorkflowNodeStatus::Succeeded);
        assert_ne!(live(&after_resume, "r").id, r_before.id);
        assert_eq!(live(&after_resume, "r").status, WorkflowNodeStatus::Running);
        assert_eq!(dispatch_counts(&executor).get("l"), Some(&1));
        assert_eq!(dispatch_counts(&executor).get("r"), Some(&2));
    });
}
