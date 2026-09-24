//! Automatic retry at its limits, through the public workflow commands and the fake ACP agent:
//! an exhausted budget and the resume after it, cancel and `get_workflow_run` during the wait,
//! and the failures that are never retried.

use super::agent_ref;
use super::workflow_resume::run_case;
use super::workflow_retry::{
    Published, TestError, attempts, prompt_failures, prompt_texts, prompted_sessions,
    single_agent_graph, wait_for_retry, worker,
};
use crate::setup::DesktopTestSetup;
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

type TestResult = Result<(), TestError>;

/// The fake agent with `prompt`, under the given retry policy.
fn agent_config(prompt: &str, retry: Value) -> Value {
    json!({
        "executor": {"agentCli": agent_ref(), "modelId": "anthropic/claude-sonnet-4"},
        "prompt": prompt,
        "retry": retry
    })
}

fn retry(enabled: bool, max_retries: u32, initial_delay_seconds: u32) -> Value {
    json!({
        "enabled": enabled,
        "maxRetries": max_retries,
        "initialDelaySeconds": initial_delay_seconds
    })
}

/// The failed-attempt history of `worker` session failures with the given attempt numbers.
fn session_attempts(numbers: &[u32]) -> Vec<(String, u32, String)> {
    numbers
        .iter()
        .map(|number| ("worker".to_string(), *number, "session".to_string()))
        .collect()
}

/// E5: two session failures spend the one-retry budget and fail the run. A resume from the
/// failure starts a fresh budget but keeps counting attempts, so the resumed attempt 3 fails,
/// is retried as attempt 4, and the run succeeds.
#[test]
fn an_exhausted_retry_fails_the_run_and_a_resume_keeps_counting_attempts() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let published = Published::new(
            &setup,
            single_agent_graph(agent_config("[fail-times:3] build it", retry(true, 1, 0))),
        )?;
        let run_id = published.start_run(None)?;

        let failed = published.settle(&run_id, WorkflowRunStatus::Failed).await?;
        let (row, payload) = worker(&failed)?;
        assert_eq!(
            (
                row.status,
                &payload["error_detail"]["kind"],
                &payload["error_detail"]["attempt"],
                &payload["auto_retry"],
                payload.get("retry_wait")
            ),
            (
                WorkflowNodeStatus::Failed,
                &json!("session"),
                &json!(2),
                &json!({"retry": 1, "max_retries": 1}),
                None
            )
        );
        assert_eq!(attempts(&failed), session_attempts(&[1]));
        assert!(
            published
                .runs
                .preview_resume(PreviewWorkflowRunResumeRequest {
                    run_id: run_id.clone(),
                })?
                .resumable
        );

        published
            .runs
            .resume_from_failure(ResumeWorkflowRunRequest {
                run_id: run_id.clone(),
                rollback: Some(ResumeRollbackMode::Keep),
                snapshot_id: None,
            })?;
        let succeeded = published
            .settle(&run_id, WorkflowRunStatus::Succeeded)
            .await?;
        let (row, payload) = worker(&succeeded)?;
        assert_eq!(
            (row.status, &payload["auto_retry"]),
            (
                WorkflowNodeStatus::Succeeded,
                &json!({"retry": 1, "max_retries": 1})
            )
        );
        assert_eq!(attempts(&succeeded), session_attempts(&[1, 2, 3]));
        assert_eq!(
            prompt_failures(&published.package_root),
            "[fail-times:3]\n".repeat(3)
        );
        assert_eq!(prompted_sessions(&published.package_root).len(), 4);
        Ok(())
    })
}

/// E6: cancelling while the worker waits for its retry ends the run at once, and the retry never
/// starts. "Never" needs the deadline passed with the timer fired: a second run of the workflow,
/// started only after the cancel, waits for a retry due later on the same runtime's timer, so
/// its successful retry shows the first run's timer has already fired and started nothing.
#[test]
fn cancelling_during_the_wait_ends_the_run_and_the_retry_never_starts() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        // The marker counts across runs: each run's first prompt fails, and the second run's
        // retry is the first prompt let through.
        let published = Published::new(
            &setup,
            single_agent_graph(agent_config("[fail-times:2] build it", retry(true, 2, 1))),
        )?;
        let cancelled_id = published.start_run(None)?;
        let (_, wait) = wait_for_retry(&published, &cancelled_id).await?;
        let due_at = wait["due_at"].as_i64().ok_or("retry_wait without due_at")?;
        let response = published
            .runs
            .cancel(CancelWorkflowRunRequest {
                run_id: cancelled_id.clone(),
            })
            .await?;
        assert_eq!(response.run.status, WorkflowRunStatus::Cancelled);
        let (cancelled, payload) = worker(&published.detail(&cancelled_id)?)?;
        assert_eq!(
            (
                cancelled.status,
                cancelled.started_at,
                payload.get("retry_wait")
            ),
            (WorkflowNodeStatus::Cancelled, None, None)
        );

        let later_id = published.start_run(None)?;
        let later = published
            .settle(&later_id, WorkflowRunStatus::Succeeded)
            .await?;
        let (later_retry, _) = worker(&later)?;
        assert!(
            later_retry
                .started_at
                .is_some_and(|started| started > due_at),
            "{later_retry:?} started before the cancelled retry was due at {due_at}"
        );

        let after = published.detail(&cancelled_id)?;
        assert_eq!(
            (after.run.status, worker(&after)?.0),
            (WorkflowRunStatus::Cancelled, cancelled)
        );
        assert_eq!(
            prompt_failures(&published.package_root),
            "[fail-times:2]\n".repeat(2)
        );
        // The cancelled run's failed prompt, then the later run's failed prompt and its retry.
        assert_eq!(prompted_sessions(&published.package_root).len(), 3);
        Ok(())
    })
}

/// E7: during the wait, `get_workflow_run` shows the waiting attempt (running, not started, with
/// its `retry_wait`) and the failed attempt it replaced, whose `sourceChain` carries the agent's
/// own error; the retry then succeeds as that same row and the history does not change.
#[test]
fn get_workflow_run_during_the_wait_shows_the_waiting_attempt_and_the_failed_one() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let published = Published::new(
            &setup,
            single_agent_graph(agent_config("[fail-times:1] build it", retry(true, 2, 1))),
        )?;
        let run_id = published.start_run(None)?;

        let (waiting, wait) = wait_for_retry(&published, &run_id).await?;
        let (waiting_row, _) = worker(&waiting)?;
        assert_eq!(
            (
                waiting.run.status,
                waiting_row.status,
                waiting_row.started_at,
                waiting_row.session_id.clone()
            ),
            (
                WorkflowRunStatus::Running,
                WorkflowNodeStatus::Running,
                None,
                None
            )
        );
        let failed = waiting.failed_attempts.clone().unwrap_or_default();
        assert_eq!(attempts(&waiting), session_attempts(&[1]));
        assert!(
            failed[0]
                .source_chain
                .iter()
                .any(|line| line.contains("fake agent failed prompt 1 of 1 on purpose")),
            "{:?}",
            failed[0]
        );
        let scheduled_at = wait["scheduled_at"]
            .as_i64()
            .ok_or("retry_wait without scheduled_at")?;
        assert_eq!(
            wait,
            json!({
                "attempt": 2,
                "max_attempt": 3,
                "retry": 1,
                "max_retries": 2,
                "delay_ms": 1000,
                "scheduled_at": scheduled_at,
                "due_at": scheduled_at + 1000,
                "previous_node_run_id": failed[0].node_run_id
            })
        );

        let succeeded = published
            .settle(&run_id, WorkflowRunStatus::Succeeded)
            .await?;
        let (retried, _) = worker(&succeeded)?;
        assert_eq!(retried.id, waiting_row.id);
        assert!(
            retried
                .started_at
                .is_some_and(|started| started >= scheduled_at + 1000),
            "{retried:?}"
        );
        assert_eq!(succeeded.failed_attempts, Some(failed));
        Ok(())
    })
}

/// E8: a prompt that references an undeclared variable fails before any session starts. That
/// kind is never retried, so even a policy with three immediate retries fails the run at once,
/// with no wait and `auto_retryable` false.
#[test]
fn a_prompt_template_failure_fails_the_run_at_once_under_an_enabled_policy() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let published = Published::new(
            &setup,
            single_agent_graph(agent_config(
                "summarize {{#start.missing#}}",
                retry(true, 3, 0),
            )),
        )?;
        let run_id = published.start_run(None)?;

        let failed = published.settle(&run_id, WorkflowRunStatus::Failed).await?;
        let (row, payload) = worker(&failed)?;
        let detail = &payload["error_detail"];
        assert_eq!(
            (
                row.status,
                &detail["kind"],
                &detail["attempt"],
                &detail["auto_retryable"],
                payload.get("retry_wait"),
                payload.get("auto_retry")
            ),
            (
                WorkflowNodeStatus::Failed,
                &json!("prompt_template"),
                &json!(1),
                &json!(false),
                None,
                None
            )
        );
        assert_eq!(attempts(&failed), Vec::new());
        assert_eq!(
            prompted_sessions(&published.package_root),
            Vec::<String>::new()
        );
        Ok(())
    })
}

/// E9: with the run's `injectLastFailure` off, a structured-output failure is still retried, but
/// the retry asks the agent exactly what the first attempt asked, with no failure block. The
/// fake agent answers prose again, so the run fails with the budget spent.
#[test]
fn with_inject_last_failure_off_the_retry_prompt_has_no_failure_block() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let mut config = agent_config("answer in JSON", retry(true, 1, 0));
        config["outputContract"] = json!({
            "type": "structured",
            "textExposure": "includeFinalText",
            "schema": {
                "type": "object",
                "properties": {"ok": {"type": "boolean"}},
                "required": ["ok"]
            }
        });
        let published = Published::new(&setup, single_agent_graph(config))?;
        let run_id = published.start_run(Some(false))?;

        let failed = published.settle(&run_id, WorkflowRunStatus::Failed).await?;
        let prompts = prompt_texts(&published.package_root)?;
        assert_eq!(prompts.len(), 2, "{prompts:?}");
        assert!(!prompts[1].contains("Previous attempt"), "{}", prompts[1]);
        assert_eq!(prompts[1], prompts[0]);
        let (row, payload) = worker(&failed)?;
        assert_eq!(
            (
                row.status,
                &payload["error_detail"]["kind"],
                &payload["error_detail"]["attempt"],
                &payload["auto_retry"],
                payload.get("injected_failure_context")
            ),
            (
                WorkflowNodeStatus::Failed,
                &json!("structured_output"),
                &json!(2),
                &json!({"retry": 1, "max_retries": 1}),
                None
            )
        );
        assert_eq!(
            attempts(&failed),
            vec![("worker".to_string(), 1, "structured_output".to_string())]
        );
        Ok(())
    })
}

/// E10: with retry turned off, the first session failure fails the node and the run at once;
/// the kind itself would be retried, which `auto_retryable` still records.
#[test]
fn with_retry_off_a_session_failure_fails_the_run_at_once() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let published = Published::new(
            &setup,
            single_agent_graph(agent_config("[fail-times:1] build it", retry(false, 3, 0))),
        )?;
        let run_id = published.start_run(None)?;

        let failed = published.settle(&run_id, WorkflowRunStatus::Failed).await?;
        let (row, payload) = worker(&failed)?;
        let detail = &payload["error_detail"];
        assert_eq!(
            (
                row.status,
                &detail["kind"],
                &detail["attempt"],
                &detail["auto_retryable"],
                payload.get("retry_wait"),
                payload.get("auto_retry")
            ),
            (
                WorkflowNodeStatus::Failed,
                &json!("session"),
                &json!(1),
                &json!(true),
                None,
                None
            )
        );
        assert_eq!(attempts(&failed), Vec::new());
        assert_eq!(prompt_failures(&published.package_root), "[fail-times:1]\n");
        assert_eq!(prompted_sessions(&published.package_root).len(), 1);
        Ok(())
    })
}
