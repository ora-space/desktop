//! Checkpoint failure and worktree-content rollback coverage against a real git temp repo.

use super::checkpoint::{record_pre_node_checkpoint, take_checkpoint};
use super::rollback::{apply_rollback, plan_rollback, preview};
use super::test_fixture::{
    ClockAt, RecordingExecutor, SeqGen, bootstrap, init_git_workspace, started_run_with,
};
use ora_application::{
    FileChange, NodeFailure, NodeFailureKind, ResumeWorkflowRunResult, WorkflowRunEngine,
    WorkflowRunRepository,
};
use ora_contracts::{EmptyErrorParams, PublicError, ResumeRollbackMode};
use ora_db::{SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository};
use ora_domain::{WorkflowNodeRun, WorkflowRunId, WorkflowRunStatus};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;
use std::path::Path;
use std::process::Command;

const LINEAR_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
    {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"c"}}},
    {"id":"output","data":{"kind":"output"}}
],"edges":[
    {"source":"start","target":"a"},
    {"source":"a","target":"c"},
    {"source":"c","target":"output"}
]}"#;

const PRE_ROLLBACK_NOW: i64 = 99;

type FixtureEngine = WorkflowRunEngine<SqliteWorkflowRunEngineRepository, SeqGen, ClockAt>;

/// B7: a workspace that is not a git repository still runs the node and refuses file rollback.
#[test]
fn non_git_workspace_records_checkpoint_error_and_refuses_file_rollback() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, LINEAR_GRAPH, executor);
        let workspace_root = temp.path().join("fixture-project");
        complete(&engine, &run_id, &live(&live_nodes(&pool, &run_id), "a").id);
        let c = live(&live_nodes(&pool, &run_id), "c").clone();
        record_pre_node_checkpoint(
            &SqliteWorkflowRunEngineRepository::new(pool.clone()),
            &workspace_root,
            &run_id,
            "c",
            &c.id,
            "snapshot-1",
            40,
        )
        .unwrap();
        std::fs::write(workspace_root.join("untouched.txt"), "keep\n").unwrap();
        engine
            .fail_node(
                &run_id,
                &c.id,
                NodeFailure::new(NodeFailureKind::Session, "c failed"),
            )
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(
            live(&live_nodes(&pool, &run_id), "c")
                .payload
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload
                .get("checkpoint")
                .and_then(serde_json::Value::as_str),
            None,
            "{payload}"
        );
        assert!(
            payload["checkpoint_error"]
                .as_str()
                .is_some_and(|error| !error.is_empty()),
            "{payload}"
        );
        let response = preview(&pool, &workspace_root, &run_id).unwrap();
        assert!(response.resumable);
        assert_eq!(response.node_files_available, false);
        assert_eq!(
            response.node_files_unavailable_reason.as_deref(),
            Some("no_file_changes")
        );
        assert_eq!(response.checkpoint_available, false);
        assert_eq!(
            response.checkpoint_unavailable_reason.as_deref(),
            Some("no_checkpoint")
        );
        let plan = plan_rollback(&pool, &run_id).unwrap();
        for mode in [
            ResumeRollbackMode::Checkpoint,
            ResumeRollbackMode::NodeFiles,
        ] {
            let error = apply_rollback(&workspace_root, &plan, mode, &run_id, PRE_ROLLBACK_NOW)
                .expect_err("file rollback must be refused without a checkpoint");
            assert_eq!(
                error.public_error(),
                &PublicError::WorkflowRunNotResumable(EmptyErrorParams {})
            );
        }
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("untouched.txt")).unwrap(),
            "keep\n"
        );
        assert!(
            !Command::new("git")
                .current_dir(&workspace_root)
                .args(["rev-parse", "--is-inside-work-tree"])
                .output()
                .unwrap()
                .status
                .success()
        );
        assert_eq!(
            engine.resume_from_failure(&run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        assert_eq!(find_run(&pool, &run_id).status, WorkflowRunStatus::Running);
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("untouched.txt")).unwrap(),
            "keep\n"
        );
    });
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

fn run_git(cwd: &Path, args: &[&str]) -> std::process::Output {
    let output = Command::new("git")
        .current_dir(cwd)
        .args([
            "-c",
            "user.name=ora-test",
            "-c",
            "user.email=ora-test@example.com",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn pre_rollback_refs(root: &Path) -> Vec<String> {
    let output = run_git(root, &["for-each-ref", "refs/ora/checkpoints/"]);
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| line.contains("pre-rollback-"))
        .map(str::to_string)
        .collect()
}

fn content_changes() -> Vec<FileChange> {
    [
        "added.txt",
        "tracked.txt",
        "gone.txt",
        "name with spaces.txt",
        "中文名.txt",
        "crlf.txt",
    ]
    .into_iter()
    .map(|path| FileChange {
        path: path.to_string(),
        additions: 1,
        deletions: 0,
    })
    .collect()
}

fn seed_content_workspace(root: &Path) {
    run_git(root, &["config", "core.autocrlf", "false"]);
    std::fs::write(root.join("tracked.txt"), "orig\n").unwrap();
    std::fs::write(root.join("gone.txt"), "present\n").unwrap();
    std::fs::write(root.join("old file.txt"), "old\n").unwrap();
    std::fs::write(root.join("crlf.txt"), b"line1\r\nline2\r\n").unwrap();
    run_git(
        root,
        &["add", "tracked.txt", "gone.txt", "old file.txt", "crlf.txt"],
    );
    run_git(root, &["commit", "-m", "content seed"]);
    std::fs::write(root.join("untracked before.txt"), "survive\n").unwrap();
}

fn apply_node_edits(root: &Path) {
    std::fs::write(root.join("added.txt"), "new\n").unwrap();
    std::fs::write(root.join("tracked.txt"), "changed\n").unwrap();
    std::fs::remove_file(root.join("gone.txt")).unwrap();
    std::fs::write(root.join("name with spaces.txt"), "spaces\n").unwrap();
    std::fs::write(root.join("中文名.txt"), "zh\n").unwrap();
    std::fs::write(root.join("crlf.txt"), b"changed\r\n").unwrap();
    std::fs::write(root.join("staged.txt"), "index\n").unwrap();
    run_git(root, &["add", "staged.txt"]);
}

fn assert_index_has_staged(root: &Path) {
    let cached = run_git(root, &["diff", "--cached", "--name-only"]);
    let names = String::from_utf8(cached.stdout).unwrap();
    assert!(
        names.lines().any(|line| line == "staged.txt"),
        "index should still list staged.txt, got {names:?}"
    );
}

fn pre_rollback_contains_added(root: &Path, added: &str) {
    let refs = pre_rollback_refs(root);
    assert_eq!(refs.len(), 1, "{refs:?}");
    let oid = refs[0].split_whitespace().next().expect("oid");
    let tree = run_git(root, &["ls-tree", "-r", "--name-only", oid]);
    let names = String::from_utf8(tree.stdout).unwrap();
    assert!(
        names.lines().any(|line| line == added),
        "pre-rollback ref should contain {added}, got {names:?}"
    );
}

/// B8: `node_files` restores add/modify/delete, special names, CRLF, index, and the safety ref.
#[test]
fn node_files_rollback_restores_add_modify_delete_and_special_names() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, LINEAR_GRAPH, executor);
        let workspace_root = init_git_workspace(&temp);
        seed_content_workspace(&workspace_root);
        complete(&engine, &run_id, &live(&live_nodes(&pool, &run_id), "a").id);
        let c = live(&live_nodes(&pool, &run_id), "c").clone();
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
        apply_node_edits(&workspace_root);
        engine
            .fail_node(
                &run_id,
                &c.id,
                NodeFailure::new(NodeFailureKind::Session, "c failed")
                    .with_file_changes(content_changes()),
            )
            .unwrap();
        let plan = plan_rollback(&pool, &run_id).unwrap();
        apply_rollback(
            &workspace_root,
            &plan,
            ResumeRollbackMode::NodeFiles,
            &run_id,
            PRE_ROLLBACK_NOW,
        )
        .unwrap();
        assert!(!workspace_root.join("added.txt").exists());
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("tracked.txt")).unwrap(),
            "orig\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("gone.txt")).unwrap(),
            "present\n"
        );
        assert!(!workspace_root.join("name with spaces.txt").exists());
        assert!(!workspace_root.join("中文名.txt").exists());
        assert_eq!(
            std::fs::read(workspace_root.join("crlf.txt")).unwrap(),
            b"line1\r\nline2\r\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("untracked before.txt")).unwrap(),
            "survive\n"
        );
        assert_index_has_staged(&workspace_root);
        pre_rollback_contains_added(&workspace_root, "added.txt");
    });
}

/// B8: `checkpoint` restores the whole worktree, including special names and CRLF bytes.
#[test]
fn checkpoint_rollback_restores_the_whole_worktree_byte_for_byte() {
    with_trace_logging(|| {
        let (temp, pool) = bootstrap();
        let executor = RecordingExecutor::default();
        let (run_id, _, engine) = started_run_with(&temp, &pool, LINEAR_GRAPH, executor);
        let workspace_root = init_git_workspace(&temp);
        seed_content_workspace(&workspace_root);
        complete(&engine, &run_id, &live(&live_nodes(&pool, &run_id), "a").id);
        let c = live(&live_nodes(&pool, &run_id), "c").clone();
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
        apply_node_edits(&workspace_root);
        std::fs::write(workspace_root.join("manual after.txt"), "after\n").unwrap();
        engine
            .fail_node(
                &run_id,
                &c.id,
                NodeFailure::new(NodeFailureKind::Session, "c failed")
                    .with_file_changes(content_changes()),
            )
            .unwrap();
        let plan = plan_rollback(&pool, &run_id).unwrap();
        apply_rollback(
            &workspace_root,
            &plan,
            ResumeRollbackMode::Checkpoint,
            &run_id,
            PRE_ROLLBACK_NOW,
        )
        .unwrap();
        assert!(!workspace_root.join("added.txt").exists());
        assert!(!workspace_root.join("manual after.txt").exists());
        assert!(!workspace_root.join("name with spaces.txt").exists());
        assert!(!workspace_root.join("中文名.txt").exists());
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("tracked.txt")).unwrap(),
            "orig\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("gone.txt")).unwrap(),
            "present\n"
        );
        assert_eq!(
            std::fs::read(workspace_root.join("crlf.txt")).unwrap(),
            b"line1\r\nline2\r\n"
        );
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("untracked before.txt")).unwrap(),
            "survive\n"
        );
        assert_index_has_staged(&workspace_root);
        pre_rollback_contains_added(&workspace_root, "added.txt");
    });
}
