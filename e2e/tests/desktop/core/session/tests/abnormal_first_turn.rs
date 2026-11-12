//! Workflow-owned first turns that end abnormally through the real run driver.
//!
//! A published `start -> worker -> output` graph is started through `WorkflowRuns`; the executor
//! creates the node session, drives the prompt through the real agent runtime, and records the
//! failure through the production transitions. Retry is disabled so the assertions observe the
//! final result instead of a scheduled workflow retry.

use super::abnormal_support::{
    CRASH_MARKER, DROP_MARKER, SILENT_MARKER, open_abnormal_backend, prompt_marker_count,
    wait_for_prompt_marker,
};
use super::workflow_retry::{single_agent_graph, worker};
use super::{
    agent_ref, current_thread_runtime, install_fake_opencode_plugin, open_ready_backend,
    seed_workspace,
};
use crate::setup::DesktopTestSetup;
use ora_backend::Backend;
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const RUN_DEADLINE: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Runs one case on a current-thread runtime with an absolute failure deadline.
fn run_case(test: impl std::future::Future<Output = TestResult>) -> TestResult {
    current_thread_runtime()?.block_on(async {
        tokio::time::timeout(RUN_DEADLINE, test)
            .await
            .map_err(|_| "abnormal first-turn case exceeded its deadline")?
    })
}

/// One agent node with automatic workflow retry disabled, so an abnormal turn is terminal.
fn agent_config(prompt: &str) -> Value {
    json!({
        "executor": {"agentCli": agent_ref(), "modelId": "anthropic/claude-sonnet-4"},
        "prompt": prompt,
        "interactive": true,
        "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0}
    })
}

/// Publishes the graph and starts one run through the same handlers the run screen calls.
fn start_run(
    backend: &Backend,
    workspace_id: &str,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let workflow = backend
        .workflows()
        .create(CreateWorkflowRequest {
            name: "Abnormal first turn".to_string(),
            graph: Some(single_agent_graph(agent_config(prompt))),
        })?
        .workflow;
    backend.workflows().publish(PublishWorkflowRequest {
        workflow_id: workflow.id.clone(),
        version: Some("v1".to_string()),
    })?;
    let run = backend
        .workflow_runs()
        .create(CreateWorkflowRunRequest {
            workspace_id: workspace_id.to_string(),
            workflow_id: workflow.id,
            locale: WorkflowRunLocale::EnUs,
            snapshot_id: None,
            kickoff_input: None,
            name: None,
            inject_last_failure: None,
        })?
        .run;
    backend.workflow_runs().start(StartWorkflowRunRequest {
        run_id: run.id.clone(),
    })?;
    Ok(run.id)
}

/// Reads the run's frozen graph and proves its worker really is an interactive node.
///
/// Without this, a graph that silently parses as non-interactive would also fail on an abnormal
/// turn, but would not prove the obligation that an abnormal opening turn must not park.
fn assert_frozen_worker_is_interactive(
    backend: &Backend,
    run_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let run = backend
        .workflow_runs()
        .get(GetWorkflowRunRequest {
            run_id: run_id.to_string(),
        })?
        .run;
    let snapshot = backend
        .workflows()
        .get_snapshot(GetWorkflowSnapshotRequest {
            snapshot_id: run.snapshot_id,
        })?
        .snapshot;
    let graph: Value = serde_json::from_str(&snapshot.graph)?;
    let worker = graph["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["id"] == "worker"))
        .ok_or("frozen graph has no worker node")?;
    assert_eq!(
        worker["data"]["agentConfig"]["interactive"],
        json!(true),
        "the run must freeze an interactive worker, otherwise failure is not evidence against parking"
    );
    Ok(())
}

/// Waits for the run to reach `Failed`, failing immediately on any other terminal status.
async fn settle_failed(
    backend: &Backend,
    run_id: &str,
) -> Result<GetWorkflowRunResponse, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + RUN_DEADLINE;
    loop {
        let detail = backend.workflow_runs().get(GetWorkflowRunRequest {
            run_id: run_id.to_string(),
        })?;
        match detail.run.status {
            WorkflowRunStatus::Failed => return Ok(detail),
            WorkflowRunStatus::Succeeded | WorkflowRunStatus::Cancelled => {
                panic!(
                    "run {run_id} ended as {:?}, not Failed: {:#?}",
                    detail.run.status, detail.nodes
                )
            }
            WorkflowRunStatus::Pending
            | WorkflowRunStatus::Running
            | WorkflowRunStatus::AwaitingInput => {}
        }
        assert!(
            Instant::now() < deadline,
            "run {run_id} did not fail within {RUN_DEADLINE:?}: {:?}",
            detail.run.status
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Asserts the failed node's diagnostic and that it neither parked nor advanced the graph.
fn assert_failed_without_parking(
    detail: &GetWorkflowRunResponse,
    expected_kind: &str,
    expected_error_fragments: &[&str],
) -> Result<String, Box<dyn std::error::Error>> {
    assert_eq!(detail.run.status, WorkflowRunStatus::Failed);
    let (row, payload) = worker(detail)?;
    assert_eq!(row.status, WorkflowNodeStatus::Failed);
    assert_eq!(
        payload["error_detail"]["kind"],
        json!(expected_kind),
        "the failure report must classify this abnormal turn: {row:?}"
    );
    let error = row.error.as_deref().unwrap_or_default();
    assert!(
        expected_error_fragments
            .iter()
            .any(|fragment| error.contains(fragment)),
        "the node error must explain the failure; got {error:?}, expected one of {expected_error_fragments:?}"
    );
    assert!(
        payload.get("completion").is_none(),
        "an automatic abnormal turn must not write a completion park snapshot: {payload}"
    );
    assert!(
        detail
            .nodes
            .iter()
            .all(|node| node.status != WorkflowNodeStatus::Pending),
        "an automatic abnormal turn must fail, not wait for the user: {:#?}",
        detail.nodes
    );
    assert!(
        detail.nodes.iter().all(|node| node.node_id != "output"),
        "no successor may execute after the failed first turn: {:#?}",
        detail.nodes
    );
    assert_eq!(
        detail.failed_attempts.as_ref().map_or(0, Vec::len),
        0,
        "the disabled workflow retry must leave one terminal attempt"
    );
    assert!(
        payload.get("retry_wait").is_none(),
        "the node must not wait for a workflow retry: {payload}"
    );
    row.session_id
        .ok_or_else(|| "the failed node row has no session id".into())
}

/// Exhausted silence fails the workflow's own first turn instead of parking it.
#[test]
fn a_silent_workflow_first_turn_exhausts_inactivity_and_fails_the_node() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_abnormal_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run_id = start_run(&backend, &workspace_id, SILENT_MARKER)?;
        assert_frozen_worker_is_interactive(&backend, &run_id)?;

        let detail = settle_failed(&backend, &run_id).await?;
        assert_failed_without_parking(&detail, "session", &["prompt made no progress"])?;
        assert_eq!(
            prompt_marker_count(&package_root, SILENT_MARKER),
            4,
            "the provider must receive the initial attempt and all three re-sent attempts"
        );
        Ok(())
    })
}

/// A workflow session crash fails the node with the provider-connection diagnostic.
#[test]
fn a_crashed_workflow_first_turn_fails_the_node_without_parking() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run_id = start_run(&backend, &workspace_id, CRASH_MARKER)?;
        assert_frozen_worker_is_interactive(&backend, &run_id)?;

        let detail = settle_failed(&backend, &run_id).await?;
        // The connection supervisor can attribute the same abrupt exit as the routed
        // connection-lost failure or as the runtime-unavailable branch; both are session-level
        // crash diagnostics, never the dropped-owner-stream or timeout classifications.
        assert_failed_without_parking(
            &detail,
            "session",
            &["agent connection was lost", "agent runtime is unavailable"],
        )?;
        assert_eq!(
            prompt_marker_count(&package_root, CRASH_MARKER),
            1,
            "the crash case must not be retried as a workflow attempt: {:#?}",
            detail.nodes
        );
        Ok(())
    })
}

/// A first-turn prompt stream that ends before `Completed` fails the node, without a park.
///
/// The workflow executor owns this stream, so the test-support seam ends it after the fake agent
/// received the prompt. That exercises the executor's real missing-`Completed` branch; it does not
/// fabricate a stop cause or reuse a crash to stand in for the dropped stream.
#[test]
fn a_workflow_first_turn_without_a_terminal_event_fails_the_node() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        backend.workflow_runs().arm_test_owner_stream_drop("worker");
        let run_id = start_run(&backend, &workspace_id, DROP_MARKER)?;
        assert_frozen_worker_is_interactive(&backend, &run_id)?;
        let claimed_session = tokio::time::timeout(
            RUN_DEADLINE,
            backend.workflow_runs().wait_for_test_owner_stream_session(),
        )
        .await
        .map_err(|_| "the executor never claimed the armed worker session")?
        .ok_or("the executor did not claim the armed worker session")?;
        wait_for_prompt_marker(&package_root, DROP_MARKER).await?;
        backend
            .workflow_runs()
            .confirm_test_owner_stream_prompt(&claimed_session)?;

        let detail = settle_failed(&backend, &run_id).await?;
        let session_id = assert_failed_without_parking(
            &detail,
            "session_ended_without_stop_reason",
            &["without a stop reason"],
        )?;
        assert_eq!(
            session_id, claimed_session,
            "the dropped stream belongs to the node session the test synchronized with"
        );
        assert_eq!(
            prompt_marker_count(&package_root, DROP_MARKER),
            1,
            "the fake agent must receive the prompt before the owner drops the stream"
        );
        Ok(())
    })
}
