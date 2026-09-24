//! Resume rollback after automatic retries, against a real git worktree.
//!
//! Automatic retries do not roll files back, so an exhausted chain of attempts is rolled back as
//! one unit on a manual resume: `checkpoint` restores the worktree from before the chain's first
//! attempt, `node_files` restores the union of every attempt's files, and the sibling check
//! starts at the first attempt. A row with no chain behaves exactly as before (the #581 rollback
//! suites cover that path unchanged).

use super::checkpoint::take_checkpoint;
use super::retry_tests::{Harness, linear, retry, siblings};
use super::rollback::{apply_rollback, plan_rollback, preview};
use super::test_fixture::init_git_workspace;
use ora_application::{
    FileChange, NodeFailure, NodeFailureKind, ResumeWorkflowRunResult, retry_chain_from_payload,
};
use ora_contracts::{PreviewWorkflowRunResumeResponse, ResumeRollbackMode};
use ora_domain::{WorkflowNodeRun, WorkflowRunStatus};
use pretty_assertions::assert_eq;
use std::path::{Path, PathBuf};
use std::process::Command;

const PRE_ROLLBACK_NOW: i64 = 99_000;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
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
}

/// Turns the harness workspace into a git repository with `f1.txt` and `f2.txt` committed.
fn seeded_workspace(h: &Harness) -> PathBuf {
    let root = init_git_workspace(&h.temp);
    git(&root, &["config", "core.autocrlf", "false"]);
    write(&root, "f1.txt", "f1 orig\n");
    write(&root, "f2.txt", "f2 orig\n");
    git(&root, &["add", "f1.txt", "f2.txt"]);
    git(&root, &["commit", "-m", "seed files"]);
    root
}

fn write(root: &Path, path: &str, content: &str) {
    std::fs::write(root.join(path), content).unwrap();
}

fn read(root: &Path, path: &str) -> String {
    std::fs::read_to_string(root.join(path)).unwrap()
}

/// Takes and records the pre-node checkpoint of `attempt`, as the executor does before it runs.
fn checkpoint(h: &Harness, root: &Path, attempt: &WorkflowNodeRun) -> String {
    let oid = take_checkpoint(root, &h.run_id, &attempt.node_id, &attempt.id)
        .commit_oid
        .expect("checkpoint oid");
    h.engine
        .record_node_checkpoint(
            &attempt.id,
            "snapshot-1",
            Some(&oid),
            /*checkpoint_error*/ None,
        )
        .unwrap();
    oid
}

fn changed(path: &str) -> Vec<FileChange> {
    vec![FileChange {
        path: path.to_string(),
        additions: 1,
        deletions: 1,
    }]
}

fn session_failure(message: &str, path: &str) -> NodeFailure {
    NodeFailure::new(NodeFailureKind::Session, message).with_file_changes(changed(path))
}

/// `(node_id, started_at, checkpoint, node_file_changes paths)` of every failed node.
fn failed_nodes(
    preview: &PreviewWorkflowRunResumeResponse,
) -> Vec<(String, Option<i64>, Option<String>, Vec<String>)> {
    preview
        .failed_nodes
        .iter()
        .map(|node| {
            (
                node.node_id.clone(),
                node.started_at,
                node.checkpoint.clone(),
                node.node_file_changes
                    .iter()
                    .map(|change| change.path.clone())
                    .collect(),
            )
        })
        .collect()
}

/// Node `a` (one retry, no wait) changes `f1.txt` in attempt 1 and `f2.txt` in attempt 2 and
/// fails both times, which fails the run. Returns the worktree and attempt 1's checkpoint.
fn exhausted_chain(h: &Harness) -> (PathBuf, String) {
    let root = seeded_workspace(h);
    let first = h.running("a");
    assert_eq!(first.started_at, Some(40));
    let first_checkpoint = checkpoint(h, &root, &first);
    write(&root, "f1.txt", "attempt 1\n");
    h.fail_with(
        &first.id,
        session_failure("attempt 1 failed", "f1.txt"),
        1_000,
    );
    let second = h.running("a");
    h.wake(&second.id, 1_000);
    checkpoint(h, &root, &second);
    write(&root, "f2.txt", "attempt 2\n");
    h.fail_with(
        &second.id,
        session_failure("attempt 2 failed", "f2.txt"),
        2_000,
    );
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);
    (root, first_checkpoint)
}

/// `node_files` after an exhausted chain restores the files of every attempt from the
/// checkpoint taken before the first one, and the preview lists the chain as one node.
#[test]
fn node_files_rollback_after_exhausted_retries_restores_the_files_of_every_attempt() {
    let h = Harness::start(&linear(retry(true, 1, 0)));
    let (root, first_checkpoint) = exhausted_chain(&h);

    let preview = preview(&h.pool, &root, &h.run_id).unwrap();
    assert_eq!(
        failed_nodes(&preview),
        vec![(
            "a".to_string(),
            Some(40),
            Some(first_checkpoint),
            vec!["f1.txt".to_string(), "f2.txt".to_string()]
        )]
    );
    assert!(preview.node_files_available, "{preview:?}");
    let mut since: Vec<String> = preview.failed_nodes[0]
        .changed_since_checkpoint
        .iter()
        .map(|change| change.path.clone())
        .collect();
    since.sort();
    assert_eq!(since, vec!["f1.txt".to_string(), "f2.txt".to_string()]);

    let plan = plan_rollback(&h.pool, &h.run_id).unwrap();
    apply_rollback(
        &root,
        &plan,
        ResumeRollbackMode::NodeFiles,
        &h.run_id,
        PRE_ROLLBACK_NOW,
    )
    .unwrap();
    assert_eq!(
        (read(&root, "f1.txt"), read(&root, "f2.txt")),
        ("f1 orig\n".to_string(), "f2 orig\n".to_string())
    );
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
}

/// `checkpoint` after an exhausted chain restores the worktree from before the first attempt,
/// not from before the last one (which already contains attempt 1's edits).
#[test]
fn checkpoint_rollback_after_exhausted_retries_restores_the_worktree_before_the_first_attempt() {
    let h = Harness::start(&linear(retry(true, 1, 0)));
    let (root, first_checkpoint) = exhausted_chain(&h);
    write(&root, "late.txt", "written after the chain\n");

    let plan = plan_rollback(&h.pool, &h.run_id).unwrap();
    assert_eq!(
        (plan.checkpoint_available, plan.checkpoint_oid.as_deref()),
        (true, Some(first_checkpoint.as_str()))
    );
    apply_rollback(
        &root,
        &plan,
        ResumeRollbackMode::Checkpoint,
        &h.run_id,
        PRE_ROLLBACK_NOW,
    )
    .unwrap();
    assert_eq!(
        (
            read(&root, "f1.txt"),
            read(&root, "f2.txt"),
            root.join("late.txt").exists()
        ),
        ("f1 orig\n".to_string(), "f2 orig\n".to_string(), false)
    );
}

/// A sibling that finished after the chain's first attempt started, but before its last attempt
/// did, would lose its files to a whole-worktree restore, so checkpoint rollback is refused.
#[test]
fn a_sibling_active_after_the_first_attempt_blocks_checkpoint_rollback_of_the_chain() {
    let h = Harness::start(&siblings(retry(true, 1, 0), retry(false, 0, 0)));
    let root = seeded_workspace(&h);
    let first = h.running("a");
    checkpoint(&h, &root, &first);
    h.complete(&h.running("b").id, "b done", 500);
    h.fail_with(
        &first.id,
        session_failure("attempt 1 failed", "f1.txt"),
        1_000,
    );
    let second = h.running("a");
    h.wake(&second.id, 1_000);
    checkpoint(&h, &root, &second);
    h.fail_with(
        &second.id,
        session_failure("attempt 2 failed", "f2.txt"),
        2_000,
    );
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);

    let preview = preview(&h.pool, &root, &h.run_id).unwrap();
    assert_eq!(
        (
            preview.resumable,
            preview.checkpoint_available,
            preview.checkpoint_unavailable_reason.as_deref(),
            preview.node_files_available
        ),
        (true, false, Some("siblings_ran_after_checkpoint"), true)
    );
    let plan = plan_rollback(&h.pool, &h.run_id).unwrap();
    assert!(
        apply_rollback(
            &root,
            &plan,
            ResumeRollbackMode::Checkpoint,
            &h.run_id,
            PRE_ROLLBACK_NOW,
        )
        .is_err()
    );
}

/// A wait the run abandoned never ran, so it needs no checkpoint of its own: its rollback is the
/// attempt it replaced, and `node_files` restores that attempt's files next to the sibling's.
#[test]
fn an_abandoned_wait_rolls_back_the_attempt_it_replaced() {
    let h = Harness::start(&siblings(retry(true, 1, 30), retry(false, 0, 0)));
    let root = seeded_workspace(&h);
    let a_first = h.running("a");
    let a_checkpoint = checkpoint(&h, &root, &a_first);
    let b = h.running("b");
    let b_checkpoint = checkpoint(&h, &root, &b);
    write(&root, "f1.txt", "a attempt 1\n");
    h.fail_with(&a_first.id, session_failure("a failed", "f1.txt"), 1_000);
    write(&root, "f2.txt", "b attempt 1\n");
    h.fail_with(&b.id, session_failure("b failed", "f2.txt"), 2_000);
    let abandoned = h.rows_of("a").pop().unwrap();
    assert_eq!(
        (abandoned.started_at, abandoned.error.as_deref()),
        (None, Some(r#"{"reason":"retry_abandoned"}"#))
    );

    let preview = preview(&h.pool, &root, &h.run_id).unwrap();
    let mut nodes = failed_nodes(&preview);
    nodes.sort();
    assert_eq!(
        nodes,
        vec![
            (
                "a".to_string(),
                Some(40),
                Some(a_checkpoint),
                vec!["f1.txt".to_string()]
            ),
            (
                "b".to_string(),
                Some(40),
                Some(b_checkpoint),
                vec!["f2.txt".to_string()]
            ),
        ]
    );
    assert!(preview.node_files_available, "{preview:?}");
    let plan = plan_rollback(&h.pool, &h.run_id).unwrap();
    apply_rollback(
        &root,
        &plan,
        ResumeRollbackMode::NodeFiles,
        &h.run_id,
        PRE_ROLLBACK_NOW,
    )
    .unwrap();
    assert_eq!(
        (read(&root, "f1.txt"), read(&root, "f2.txt")),
        ("f1 orig\n".to_string(), "f2 orig\n".to_string())
    );
}

/// An attempt of the chain that ran without a checkpoint leaves its changes unknown: `node_files`
/// is refused, while `checkpoint` still restores from before the first attempt.
#[test]
fn a_chain_attempt_without_a_checkpoint_refuses_node_files_but_keeps_checkpoint_rollback() {
    let h = Harness::start(&linear(retry(true, 1, 0)));
    let root = seeded_workspace(&h);
    let first = h.running("a");
    let first_checkpoint = checkpoint(&h, &root, &first);
    h.fail_with(
        &first.id,
        session_failure("attempt 1 failed", "f1.txt"),
        1_000,
    );
    let second = h.running("a");
    h.wake(&second.id, 1_000);
    h.engine
        .record_node_checkpoint(
            &second.id,
            "snapshot-1",
            /*checkpoint*/ None,
            Some("git snapshot failed"),
        )
        .unwrap();
    h.fail(&second.id, NodeFailureKind::Session, 2_000);

    let preview = preview(&h.pool, &root, &h.run_id).unwrap();
    assert_eq!(
        (
            preview.node_files_available,
            preview.node_files_unavailable_reason.as_deref(),
            preview.checkpoint_available,
            preview.failed_nodes[0].checkpoint.as_deref(),
            preview.failed_nodes[0].checkpoint_error.as_deref()
        ),
        (
            false,
            Some("no_file_changes"),
            true,
            Some(first_checkpoint.as_str()),
            Some("git snapshot failed")
        )
    );
}

/// A manual resume ends the chain: the attempts after it form a new chain whose baseline is the
/// first attempt after the resume, and a failure that is not retried is a chain of one that
/// rolls back exactly its own files from its own checkpoint.
#[test]
fn a_resume_starts_a_new_chain_and_an_unretried_failure_is_a_chain_of_one() {
    let h = Harness::start(&linear(retry(true, 1, 0)));
    let (root, _) = exhausted_chain(&h);
    h.set_now(3_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let third = h.running("a");
    assert_eq!(third.started_at, Some(3_000));
    let third_checkpoint = checkpoint(&h, &root, &third);
    write(&root, "f2.txt", "attempt 3\n");
    h.fail_with(
        &third.id,
        NodeFailure::new(NodeFailureKind::PromptTemplate, "bad template")
            .with_file_changes(changed("f2.txt")),
        4_000,
    );
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);

    let preview = preview(&h.pool, &root, &h.run_id).unwrap();
    assert_eq!(
        failed_nodes(&preview),
        vec![(
            "a".to_string(),
            Some(3_000),
            Some(third_checkpoint.clone()),
            vec!["f2.txt".to_string()]
        )]
    );
    let plan = plan_rollback(&h.pool, &h.run_id).unwrap();
    assert_eq!(plan.checkpoint_oid, Some(third_checkpoint));
    apply_rollback(
        &root,
        &plan,
        ResumeRollbackMode::NodeFiles,
        &h.run_id,
        PRE_ROLLBACK_NOW,
    )
    .unwrap();
    // Attempt 3's checkpoint already held the earlier chain's edits; only its own file reverts.
    assert_eq!(
        (read(&root, "f1.txt"), read(&root, "f2.txt")),
        ("attempt 1\n".to_string(), "attempt 2\n".to_string())
    );
}

/// A cancel during the wait leaves a cancelled row that never ran; both rollback modes still
/// work from the attempt it replaced instead of being refused for a missing checkpoint.
#[test]
fn a_cancel_during_the_wait_keeps_both_rollback_modes_for_the_replaced_attempt() {
    for mode in [
        ResumeRollbackMode::NodeFiles,
        ResumeRollbackMode::Checkpoint,
    ] {
        let h = Harness::start(&linear(retry(true, 2, 30)));
        let root = seeded_workspace(&h);
        let first = h.running("a");
        let first_checkpoint = checkpoint(&h, &root, &first);
        write(&root, "f1.txt", "attempt 1\n");
        write(&root, "added.txt", "new\n");
        h.fail_with(
            &first.id,
            NodeFailure::new(NodeFailureKind::Session, "attempt 1 failed").with_file_changes(
                ["f1.txt", "added.txt"]
                    .into_iter()
                    .flat_map(changed)
                    .collect(),
            ),
            1_000,
        );
        h.set_now(2_000);
        assert_eq!(
            h.engine.cancel(&h.run_id).unwrap(),
            ora_application::CancelWorkflowRunResult::Cancelled
        );

        let preview = preview(&h.pool, &root, &h.run_id).unwrap();
        assert_eq!(
            (
                failed_nodes(&preview),
                preview.node_files_available,
                preview.checkpoint_available
            ),
            (
                vec![(
                    "a".to_string(),
                    Some(40),
                    Some(first_checkpoint.clone()),
                    vec!["f1.txt".to_string(), "added.txt".to_string()]
                )],
                true,
                true
            ),
            "{mode:?}"
        );
        let plan = plan_rollback(&h.pool, &h.run_id).unwrap();
        apply_rollback(&root, &plan, mode, &h.run_id, PRE_ROLLBACK_NOW).unwrap();
        assert_eq!(
            (read(&root, "f1.txt"), root.join("added.txt").exists()),
            ("f1 orig\n".to_string(), false),
            "{mode:?}"
        );
    }
}

/// Every retry appends the attempt it replaces to `payload.retry_chain`, oldest first, and the
/// chain survives the attempt's own failure; a manual resume starts an empty chain.
#[test]
fn the_retry_chain_lists_every_replaced_attempt_until_a_manual_resume() {
    let h = Harness::start(&linear(retry(true, 2, 0)));
    let chain_of = |row: &WorkflowNodeRun| retry_chain_from_payload(row.payload.as_deref());
    let first = h.running("a");
    assert_eq!(chain_of(&first), Vec::<String>::new());
    h.fail(&first.id, NodeFailureKind::Session, 1_000);
    let second = h.running("a");
    assert_eq!(chain_of(&second), vec![first.id.to_string()]);
    h.wake(&second.id, 1_000);
    h.fail(&second.id, NodeFailureKind::Session, 2_000);
    let third = h.running("a");
    assert_eq!(
        chain_of(&third),
        vec![first.id.to_string(), second.id.to_string()]
    );
    h.wake(&third.id, 2_000);
    h.fail(&third.id, NodeFailureKind::Session, 3_000);
    let exhausted = h.rows_of("a").pop().unwrap();
    assert_eq!(
        (exhausted.id.clone(), chain_of(&exhausted)),
        (
            third.id.clone(),
            vec![first.id.to_string(), second.id.to_string()]
        )
    );

    h.set_now(4_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let fourth = h.running("a");
    assert_eq!(chain_of(&fourth), Vec::<String>::new());
    h.fail(&fourth.id, NodeFailureKind::Session, 5_000);
    assert_eq!(chain_of(&h.running("a")), vec![fourth.id.to_string()]);
}
