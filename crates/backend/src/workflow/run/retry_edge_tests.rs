//! Automatic retry around the run's control operations (cancel, resume, restart, delete, a
//! snapshot switch), the policy's delay table, and which failure each attempt is told about.

use super::retry_tests::{Harness, linear, retry, siblings};
use super::rollback::preview;
use super::snapshot_switch::switch_if_requested;
use ora_application::{
    CancelWorkflowRunResult, DeleteWorkflowRunResult, NodeAutoRetry, NodeFailure, NodeFailureKind,
    PublishSnapshotResult, RestartWorkflowRunResult, ResumeWorkflowRunResult, UpdateDraftResult,
    WorkflowRepository, WorkflowRunRepository, running_row_blocks_resume,
};
use ora_db::{SqliteWorkflowRepository, SqliteWorkflowRunRepository};
use ora_domain::{
    WorkflowId, WorkflowNodeRun, WorkflowNodeStatus, WorkflowRunStatus, WorkflowSnapshotId,
};
use ora_logging::with_recorded_trace_logging;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tracing::Level;
use tracing_subscriber::layer::{Context, Layer};

/// `(attempt, max_attempt, retry, max_retries, delay_ms)` of a waiting row.
fn wait_tuple(row: &WorkflowNodeRun) -> Option<(u32, u32, u32, u32, i64)> {
    Harness::wait_of(row).map(|wait| {
        (
            wait.attempt,
            wait.max_attempt,
            wait.retry,
            wait.max_retries,
            wait.delay_ms,
        )
    })
}

/// Fails the running attempt of `node_id` with a session failure at every step and starts each
/// retry at its deadline, until the budget is spent. Returns the waits in order and the time
/// after the final failure.
fn exhaust(h: &Harness, node_id: &str, mut now: i64) -> (Vec<(u32, u32, u32, u32, i64)>, i64) {
    let mut waits = Vec::new();
    loop {
        h.fail(&h.running(node_id).id, NodeFailureKind::Session, now);
        let Some(row) = h
            .rows_of(node_id)
            .into_iter()
            .find(|row| Harness::wait_of(row).is_some())
        else {
            return (waits, now);
        };
        let wait = Harness::wait_of(&row).unwrap();
        waits.push(wait_tuple(&row).unwrap());
        now = wait.due_at;
        h.wake(&row.id, now);
        now += 1;
    }
}

/// Records the level of every logging event, to prove a path logs no warning or error.
#[derive(Clone, Default)]
struct LevelRecorder(Arc<Mutex<Vec<Level>>>);

impl LevelRecorder {
    fn warnings_and_errors(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|level| **level == Level::WARN || **level == Level::ERROR)
            .count()
    }
}

impl<S: tracing::Subscriber> Layer<S> for LevelRecorder {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        self.0.lock().unwrap().push(*event.metadata().level());
    }
}

/// Cancel and the retry timer both take the run lock, so they only ever run one after the
/// other. Whichever comes first, nothing is left running and nothing starts afterwards.
#[test]
fn cancel_and_the_retry_timer_in_either_order_leave_nothing_running() {
    // Cancel first: the waiting row is cancelled and the timer's wake starts nothing.
    let h = Harness::start(&linear(json!({})));
    let first = h.running("a");
    h.fail(&first.id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("a");
    h.set_now(5_000);
    assert_eq!(
        h.engine.cancel(&h.run_id).unwrap(),
        CancelWorkflowRunResult::Cancelled
    );
    h.wake(&waiting.id, 11_000);
    let row = h.rows_of("a").pop().unwrap();
    assert_eq!(
        (
            row.id,
            row.status,
            row.started_at,
            h.run().status,
            h.dispatched("a")
        ),
        (
            waiting.id,
            WorkflowNodeStatus::Cancelled,
            None,
            WorkflowRunStatus::Cancelled,
            vec![first.id.to_string()]
        )
    );

    // Timer first: the retry starts, then cancel stops it; its session's late failure and a
    // repeated wake change nothing and arm no new timer.
    let h = Harness::start(&linear(json!({})));
    let first = h.running("a");
    h.fail(&first.id, NodeFailureKind::Session, 1_000);
    let started = h.running("a");
    h.wake(&started.id, 11_000);
    h.set_now(12_000);
    assert_eq!(
        h.engine.cancel(&h.run_id).unwrap(),
        CancelWorkflowRunResult::Cancelled
    );
    let armed_after_cancel = h.armed();
    h.fail(&started.id, NodeFailureKind::Session, 13_000);
    h.wake(&started.id, 14_000);
    let row = h.rows_of("a").pop().unwrap();
    assert_eq!(
        (
            row.id,
            row.status,
            row.started_at,
            h.run().status,
            h.dispatched("a"),
            h.armed()
        ),
        (
            started.id.clone(),
            WorkflowNodeStatus::Cancelled,
            Some(11_000),
            WorkflowRunStatus::Cancelled,
            vec![first.id.to_string(), started.id.to_string()],
            armed_after_cancel
        )
    );
}

/// Two waits own separate deadlines. When the one due first is retried and fails for good, the
/// run fails and the other wait is abandoned before its deadline; its late wake starts nothing
/// and a resume reruns both nodes with fresh budgets.
#[test]
fn an_exhausted_wait_abandons_a_later_wait_of_another_node() {
    let h = Harness::start(&siblings(retry(true, 1, 30), retry(true, 1, 10)));
    let first_a = h.running("a");
    h.fail(&first_a.id, NodeFailureKind::Session, 1_000);
    let waiting_a = h.running("a");
    h.fail(&h.running("b").id, NodeFailureKind::Session, 2_000);
    let waiting_b = h.running("b");
    assert_eq!(
        h.armed(),
        vec![
            (waiting_a.id.to_string(), 31_000),
            (waiting_b.id.to_string(), 12_000)
        ]
    );

    h.wake(&waiting_b.id, 12_000);
    h.fail(&waiting_b.id, NodeFailureKind::Session, 13_000);
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);
    let abandoned = h.rows_of("a").pop().unwrap();
    assert_eq!(
        (
            abandoned.id.clone(),
            abandoned.status,
            abandoned.started_at,
            Harness::wait_of(&abandoned),
            abandoned
                .error
                .as_deref()
                .map(|error| serde_json::from_str::<Value>(error).unwrap())
        ),
        (
            waiting_a.id.clone(),
            WorkflowNodeStatus::Cancelled,
            None,
            None,
            Some(json!({"reason": "retry_abandoned"}))
        )
    );

    h.wake(&waiting_a.id, 31_000);
    assert_eq!(h.dispatched("a"), vec![first_a.id.to_string()]);

    h.set_now(40_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::Resumed
    );
    let (a, b) = (h.running("a"), h.running("b"));
    assert_eq!(
        (
            a.started_at,
            NodeAutoRetry::from_payload(a.payload.as_deref()),
            b.started_at,
            NodeAutoRetry::from_payload(b.payload.as_deref())
        ),
        (Some(40_000), None, Some(40_000), None)
    );
}

/// A waiting attempt keeps the run `Running`, so a resume is refused and the resume preview
/// reports the run as not resumable.
#[test]
fn resume_is_refused_while_an_attempt_waits_and_the_preview_says_not_resumable() {
    let h = Harness::start(&linear(json!({})));
    h.fail(&h.running("a").id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("a");
    assert!(running_row_blocks_resume(&waiting.node_type));

    h.set_now(2_000);
    assert_eq!(
        h.engine.resume_from_failure(&h.run_id).unwrap(),
        ResumeWorkflowRunResult::NotResumable
    );
    let preview = preview(&h.pool, &h.temp.path().join("fixture-project"), &h.run_id).unwrap();
    assert_eq!(
        (
            preview.resumable,
            preview.failed_nodes.len(),
            preview.node_files_available,
            preview.checkpoint_available,
            preview.checkpoint_unavailable_reason
        ),
        (false, 0, false, false, Some("not_resumable".to_string()))
    );
    assert_eq!(
        (h.running("a").id, h.run().status),
        (waiting.id, WorkflowRunStatus::Running)
    );
}

/// Publishes `graph` as snapshot `snapshot-2` (`v2`) of the harness workflow.
fn publish_v2(h: &Harness, graph: &str) {
    let repository = SqliteWorkflowRepository::new(h.pool.clone());
    let workflow_id = WorkflowId::new("workflow-1");
    assert!(matches!(
        repository
            .update_draft(&workflow_id, graph.to_string(), 50)
            .unwrap(),
        UpdateDraftResult::Updated(_)
    ));
    assert!(matches!(
        repository
            .publish_snapshot(
                &workflow_id,
                WorkflowSnapshotId::new("snapshot-2"),
                "v2".to_string(),
                50
            )
            .unwrap(),
        PublishSnapshotResult::Published(_)
    ));
}

/// The policy is read from the run's snapshot when a failure arrives, so a resume that switches
/// to a snapshot with another policy retries under the new one: a bigger budget with a shorter
/// delay, or no retry at all. The budget starts fresh and attempt numbers keep counting.
#[test]
fn a_resume_that_switches_snapshots_retries_under_the_new_policy() {
    for (new_policy, expected_wait) in [
        (retry(true, 3, 1), Some((4, 6, 1, 3, 1_000))),
        (retry(false, 0, 0), None),
    ] {
        let h = Harness::start(&linear(retry(true, 1, 10)));
        let (waits, _) = exhaust(&h, "a", 1_000);
        assert_eq!(waits, vec![(2, 2, 1, 1, 10_000)]);
        assert_eq!(h.run().status, WorkflowRunStatus::Failed);

        publish_v2(&h, &linear(new_policy.clone()));
        let skills_root = h.temp.path().join("skills");
        std::fs::create_dir_all(&skills_root).unwrap();
        assert!(
            switch_if_requested(
                &h.pool,
                &skills_root,
                &h.temp.path().join("fixture-project"),
                &h.run_id,
                Some("snapshot-2"),
                60,
            )
            .unwrap()
        );
        h.set_now(20_000);
        assert_eq!(
            h.engine.resume_from_failure(&h.run_id).unwrap(),
            ResumeWorkflowRunResult::Resumed
        );
        h.fail(&h.running("a").id, NodeFailureKind::Session, 21_000);
        let row = h.rows_of("a").pop().unwrap();
        let expected_status = match expected_wait {
            Some(_) => WorkflowRunStatus::Running,
            None => WorkflowRunStatus::Failed,
        };
        assert_eq!(
            (
                h.run().snapshot_id,
                wait_tuple(&row),
                h.run().status,
                Harness::error_detail(&row)["attempt"].clone()
            ),
            (
                WorkflowSnapshotId::new("snapshot-2"),
                expected_wait,
                expected_status,
                match expected_wait {
                    Some(_) => Value::Null,
                    None => json!(3),
                }
            ),
            "{new_policy}"
        );
    }
}

/// A restart from scratch after the budget is spent starts a fresh budget (the new first attempt
/// carries no retry marker), while attempt numbers and the failed-attempt history keep counting
/// across the restart: the restart soft-deletes the old rows, and both count every soft-deleted
/// failed row of the node in the run.
#[test]
fn a_restart_after_exhaustion_starts_a_fresh_budget_and_keeps_counting_attempts() {
    let h = Harness::start(&linear(retry(true, 1, 10)));
    let (waits, _) = exhaust(&h, "a", 1_000);
    assert_eq!(waits, vec![(2, 2, 1, 1, 10_000)]);
    assert_eq!(h.run().status, WorkflowRunStatus::Failed);

    h.set_now(20_000);
    assert_eq!(
        h.engine.restart(&h.run_id).unwrap(),
        RestartWorkflowRunResult::Restarted
    );
    let restarted = h.running("a");
    assert_eq!(
        (
            restarted.started_at,
            NodeAutoRetry::from_payload(restarted.payload.as_deref())
        ),
        (Some(20_000), None)
    );
    let (waits, _) = exhaust(&h, "a", 21_000);
    assert_eq!(waits, vec![(4, 4, 1, 1, 10_000)]);
    assert_eq!(
        Harness::error_detail(&h.rows_of("a")[0])["attempt"],
        json!(4)
    );
    assert_eq!(
        h.detail()
            .failed_attempts
            .iter()
            .map(|row| Harness::error_detail(row)["attempt"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(2), json!(3)]
    );
}

/// A run is never deleted while a node waits, because a waiting row keeps the run active. After
/// a cancel the run can be deleted, and the timer's late wake is a quiet no-op: it returns `Ok`
/// (the timer logs only errors), starts nothing, and logs no warning or error.
#[test]
fn a_run_is_deleted_only_after_its_wait_ends_and_the_late_wake_is_a_quiet_no_op() {
    let h = Harness::start(&linear(json!({})));
    let first = h.running("a");
    h.fail(&first.id, NodeFailureKind::Session, 1_000);
    let waiting = h.running("a");
    let runs = SqliteWorkflowRunRepository::new(h.pool.clone());
    assert_eq!(
        runs.soft_delete_run(&h.run_id, 2_000).unwrap(),
        DeleteWorkflowRunResult::ActiveRun
    );

    h.set_now(3_000);
    assert_eq!(
        h.engine.cancel(&h.run_id).unwrap(),
        CancelWorkflowRunResult::Cancelled
    );
    assert_eq!(
        runs.soft_delete_run(&h.run_id, 4_000).unwrap(),
        DeleteWorkflowRunResult::Deleted
    );
    assert_eq!(runs.find_run(&h.run_id).unwrap(), None);

    let recorder = LevelRecorder::default();
    h.set_now(11_000);
    let (wake, warnings_during_wake) = with_recorded_trace_logging(recorder.clone(), || {
        let wake = h.engine.wake_retry(&h.run_id, &waiting.id);
        let warnings = recorder.warnings_and_errors();
        // A control warning proves the recorder sees this thread's events.
        ora_logging::ora_warn!("control warning");
        (wake, warnings)
    });
    assert_eq!(
        (
            wake.is_ok(),
            warnings_during_wake,
            recorder.warnings_and_errors(),
            h.dispatched("a")
        ),
        (true, 0, 1, vec![first.id.to_string()])
    );
}

/// An interactive agent never retries: every retryable kind fails its only attempt at once,
/// without a waiting row, a timer, or a soft-deleted attempt.
#[test]
fn an_interactive_node_fails_at_once_on_every_retryable_kind() {
    for kind in [
        NodeFailureKind::Session,
        NodeFailureKind::SessionEndedWithoutStopReason,
        NodeFailureKind::SessionBindingRejected,
        NodeFailureKind::StructuredOutput,
        NodeFailureKind::AgentRefusal,
        NodeFailureKind::UnknownStopReason,
    ] {
        let h = Harness::start(&linear(
            json!({"interactive": true, "retry": {"enabled": true, "maxRetries": 5, "initialDelaySeconds": 1}}),
        ));
        let first = h.running("a");
        h.fail(&first.id, kind, 1_000);
        let rows = h.rows_of("a");
        assert_eq!(
            (
                rows.iter()
                    .map(|row| (row.id.clone(), row.status, Harness::wait_of(row)))
                    .collect::<Vec<_>>(),
                h.armed(),
                h.detail().failed_attempts.len(),
                h.run().status
            ),
            (
                vec![(first.id.clone(), WorkflowNodeStatus::Failed, None)],
                Vec::new(),
                0,
                WorkflowRunStatus::Failed
            ),
            "{kind:?}"
        );
    }
}

/// The wait before every retry for each budget of 1 to 5 retries and each initial delay of 0, 1,
/// 10, and 300 seconds: it doubles from the initial delay and is capped at 600 seconds, and the
/// failure after the last retry fails the run.
#[test]
fn every_budget_and_initial_delay_waits_the_documented_sequence() {
    let sequences: [(u32, [i64; 5]); 4] = [
        (0, [0, 0, 0, 0, 0]),
        (1, [1, 2, 4, 8, 16]),
        (10, [10, 20, 40, 80, 160]),
        (300, [300, 600, 600, 600, 600]),
    ];
    for max_retries in 1..=5_u32 {
        for (initial, seconds) in sequences {
            let h = Harness::start(&linear(retry(true, max_retries, initial)));
            let (waits, _) = exhaust(&h, "a", 1_000);
            let expected: Vec<(u32, u32, u32, u32, i64)> = (1..=max_retries)
                .zip(seconds)
                .map(|(retry, seconds)| {
                    (
                        retry + 1,
                        max_retries + 1,
                        retry,
                        max_retries,
                        seconds * 1_000,
                    )
                })
                .collect();
            assert_eq!(
                (waits, h.dispatched("a").len(), h.run().status),
                (
                    expected,
                    usize::try_from(max_retries + 1).unwrap(),
                    WorkflowRunStatus::Failed
                ),
                "maxRetries {max_retries}, initialDelaySeconds {initial}"
            );
        }
    }
}

fn behaviour_failure(message: &str) -> NodeFailure {
    NodeFailure::new(NodeFailureKind::StructuredOutput, message)
        .with_output(Some("plain text".to_string()))
}

/// Each retry is told about the attempt right before it, and only when that attempt failed in an
/// injectable (agent behaviour) way; an older behaviour failure is never reached past a newer
/// session failure. With the run's switch off nothing is injected.
#[test]
fn each_retry_is_told_only_about_the_attempt_right_before_it() {
    // A behaviour failure after a session failure is injected.
    let h = Harness::start(&linear(retry(true, 3, 1)));
    h.fail(&h.running("a").id, NodeFailureKind::Session, 1_000);
    h.wake(&h.running("a").id, 2_000);
    assert!(!h.prompt_for("a").contains("Previous attempt"));
    h.fail_with(
        &h.running("a").id,
        behaviour_failure("second reply is not JSON"),
        3_000,
    );
    h.wake(&h.running("a").id, 5_000);
    let prompt = h.prompt_for("a");
    assert_eq!(prompt.matches("Previous attempt").count(), 1, "{prompt}");
    assert!(prompt.contains("Previous attempt (2) failed"), "{prompt}");
    assert!(prompt.contains("second reply is not JSON"), "{prompt}");

    // A session failure after a behaviour failure: the third attempt gets nothing.
    let h = Harness::start(&linear(retry(true, 3, 1)));
    h.fail_with(
        &h.running("a").id,
        behaviour_failure("first reply is not JSON"),
        1_000,
    );
    h.wake(&h.running("a").id, 2_000);
    assert!(h.prompt_for("a").contains("Previous attempt (1) failed"));
    h.fail(&h.running("a").id, NodeFailureKind::Session, 3_000);
    h.wake(&h.running("a").id, 5_000);
    let prompt = h.prompt_for("a");
    assert!(!prompt.contains("Previous attempt"), "{prompt}");

    // The run's switch off: behaviour failures retry without the block.
    let h = Harness::start(&linear(retry(true, 3, 1)));
    h.set_run_payload_key("injectLastFailure", json!(false));
    h.fail_with(&h.running("a").id, behaviour_failure("no JSON"), 1_000);
    h.wake(&h.running("a").id, 2_000);
    h.fail_with(&h.running("a").id, behaviour_failure("no JSON"), 3_000);
    h.wake(&h.running("a").id, 5_000);
    assert_eq!(h.dispatched("a").len(), 3);
    assert!(!h.prompt_for("a").contains("Previous attempt"));
}
