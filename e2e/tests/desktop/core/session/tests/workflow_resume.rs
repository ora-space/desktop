//! One full resume-from-failure flow through the public Tauri workflow commands.
//!
//! The failing agents disable automatic retry (`agentConfig.retry`) so their first failure fails
//! the run for the resume to recover; automatic retry has its own flows in `workflow_retry`.

use super::{
    agent_ref, current_thread_runtime, install_fake_opencode_plugin, main_workspace_id,
    open_ready_backend,
};
use crate::setup::DesktopTestSetup;
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;
use std::time::{Duration, Instant};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Keeps setup, the fake ACP agent, and the public commands under one TRACE-scoped runtime.
pub(super) fn run_case(test: impl std::future::Future<Output = TestResult>) -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?
            .block_on(async { tokio::time::timeout(Duration::from_secs(15), test).await? })
    })
}

/// Polls run status without blocking the current-thread runtime that drives agent dispatch.
pub(super) async fn wait_run_status(
    runs: &ora_backend::WorkflowRuns,
    run_id: &str,
    expected: WorkflowRunStatus,
) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let detail = runs.get(GetWorkflowRunRequest {
            run_id: run_id.to_string(),
        })?;
        if detail.run.status == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "run {} stayed {:?} instead of {expected:?}",
                run_id, detail.run.status
            )
            .into());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Creates a two-agent linear graph whose second node requires `{"ok": true}` JSON.
fn structured_second_node_graph() -> String {
    json!({
        "nodes": [
            {"id": "start", "data": {"kind": "start"}},
            {"id": "first", "data": {"kind": "agent", "agentConfig": {
                "executor": {"agentCli": agent_ref(), "modelId": "anthropic/claude-sonnet-4"},
                "prompt": "first"
            }}},
            {"id": "second", "data": {"kind": "agent", "agentConfig": {
                "executor": {"agentCli": agent_ref(), "modelId": "anthropic/claude-sonnet-4"},
                "prompt": "second",
                "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0},
                "outputContract": {
                    "type": "structured",
                    "textExposure": "includeFinalText",
                    "schema": {
                        "type": "object",
                        "properties": {"ok": {"type": "boolean"}},
                        "required": ["ok"]
                    }
                }
            }}},
            {"id": "output", "data": {"kind": "output"}}
        ],
        "edges": [
            {"source": "start", "target": "first"},
            {"source": "first", "target": "second"},
            {"source": "second", "target": "output"}
        ]
    })
    .to_string()
}

/// E1: fail the structured second node, resume with keep, and succeed on the injected retry.
#[test]
fn resume_from_failure_retries_the_structured_node_after_injected_context() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        backend.projects().create(CreateProjectRequest {
            name: "Resume E2E".to_string(),
            main_workspace_path: workspace.to_string_lossy().into_owned(),
        })?;
        let workspace_id = main_workspace_id(&backend)?;
        let workflow = backend
            .workflows()
            .create(CreateWorkflowRequest {
                name: "Resume linear".to_string(),
                graph: Some(structured_second_node_graph()),
            })?
            .workflow;
        backend.workflows().publish(PublishWorkflowRequest {
            workflow_id: workflow.id.clone(),
            version: Some("v1".to_string()),
        })?;
        let runs = backend.workflow_runs();
        let run = runs
            .create(CreateWorkflowRunRequest {
                workspace_id,
                workflow_id: workflow.id,
                locale: WorkflowRunLocale::EnUs,
                snapshot_id: None,
                kickoff_input: None,
                name: None,
                inject_last_failure: None,
            })?
            .run;
        runs.start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        wait_run_status(&runs, &run.id, WorkflowRunStatus::Failed).await?;
        let failed = runs.get(GetWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        let first_failed = failed
            .nodes
            .iter()
            .find(|node| node.node_id == "first")
            .ok_or("missing first node")?;
        let first_id = first_failed.id.clone();
        let first_started = first_failed.started_at;
        assert_eq!(first_failed.status, WorkflowNodeStatus::Succeeded);
        let preview = runs.preview_resume(PreviewWorkflowRunResumeRequest {
            run_id: run.id.clone(),
        })?;
        assert!(preview.resumable);
        runs.resume_from_failure(ResumeWorkflowRunRequest {
            run_id: run.id.clone(),
            rollback: Some(ResumeRollbackMode::Keep),
            snapshot_id: None,
        })?;
        wait_run_status(&runs, &run.id, WorkflowRunStatus::Succeeded).await?;
        let succeeded = runs.get(GetWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        let first_live = succeeded
            .nodes
            .iter()
            .find(|node| node.node_id == "first")
            .ok_or("missing first node after resume")?;
        assert_eq!(first_live.id, first_id);
        assert_eq!(first_live.started_at, first_started);
        let second_live = succeeded
            .nodes
            .iter()
            .find(|node| node.node_id == "second")
            .ok_or("missing second node after resume")?;
        assert_eq!(second_live.status, WorkflowNodeStatus::Succeeded);
        let payload: serde_json::Value = serde_json::from_str(
            second_live
                .payload
                .as_deref()
                .ok_or("missing second node payload")?,
        )?;
        assert!(
            payload
                .get("injected_failure_context")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains("Previous attempt")),
            "{payload}"
        );
        Ok(())
    })
}

/// A Loop whose body agent must answer `{"ok": true}` JSON; the Loop ends on the first answer.
fn loop_with_structured_writer_graph() -> String {
    json!({
        "schemaVersion": 2,
        "nodes": [
            {"id": "start", "data": {"kind": "start"}},
            {"id": "loop", "data": {"kind": "loop", "loopConfig": {
                "maxIterations": 2,
                "variables": [{"name": "draft", "valueType": "string",
                    "initial": {"kind": "constant", "value": "seed"},
                    "feedback": ["writer", "output"]}],
                "until": {"logic": "and", "conditions": [
                    {"variableSelector": ["writer", "output"], "operator": "not_empty"}
                ]},
                "outputs": [{"name": "result", "variableSelector": ["writer", "output"]}]
            }}},
            {"id": "entry", "parentId": "loop", "data": {"kind": "start", "containerId": "loop"}},
            {"id": "writer", "parentId": "loop", "data": {"kind": "agent", "containerId": "loop",
                "agentConfig": {
                    "executor": {"agentCli": agent_ref(), "modelId": "anthropic/claude-sonnet-4"},
                    "prompt": "revise the draft",
                    "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0},
                    "outputContract": {
                        "type": "structured",
                        "textExposure": "includeFinalText",
                        "schema": {
                            "type": "object",
                            "properties": {"ok": {"type": "boolean"}},
                            "required": ["ok"]
                        }
                    }
                }}},
            {"id": "output", "data": {"kind": "output", "outputs": [
                {"name": "draft", "variableSelector": ["loop", "result"]}
            ]}}
        ],
        "edges": [
            {"source": "start", "target": "loop"},
            {"source": "entry", "target": "writer"},
            {"source": "loop", "target": "output"}
        ]
    })
    .to_string()
}

/// E2: a structured failure inside a Loop round fails the Loop; resume restarts the Loop from
/// round 1 with the previous failure injected into the rerun, and only into the rerun.
#[test]
fn resume_from_failure_restarts_a_loop_round_with_injected_context() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        backend.projects().create(CreateProjectRequest {
            name: "Resume loop E2E".to_string(),
            main_workspace_path: workspace.to_string_lossy().into_owned(),
        })?;
        let workspace_id = main_workspace_id(&backend)?;
        let workflow = backend
            .workflows()
            .create(CreateWorkflowRequest {
                name: "Resume loop".to_string(),
                graph: Some(loop_with_structured_writer_graph()),
            })?
            .workflow;
        backend.workflows().publish(PublishWorkflowRequest {
            workflow_id: workflow.id.clone(),
            version: Some("v1".to_string()),
        })?;
        let runs = backend.workflow_runs();
        let run = runs
            .create(CreateWorkflowRunRequest {
                workspace_id,
                workflow_id: workflow.id,
                locale: WorkflowRunLocale::EnUs,
                snapshot_id: None,
                kickoff_input: None,
                name: None,
                inject_last_failure: None,
            })?
            .run;
        runs.start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        wait_run_status(&runs, &run.id, WorkflowRunStatus::Failed).await?;
        let failed = runs.get(GetWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        let loop_failed = failed
            .nodes
            .iter()
            .find(|node| node.node_id == "loop")
            .ok_or("missing loop node")?;
        assert_eq!(loop_failed.status, WorkflowNodeStatus::Failed);
        let writer_failed = failed
            .nodes
            .iter()
            .find(|node| node.node_id == "writer")
            .ok_or("missing writer node")?;
        assert_eq!(writer_failed.status, WorkflowNodeStatus::Failed);
        let loop_failed_id = loop_failed.id.clone();
        let writer_failed_id = writer_failed.id.clone();
        let preview = runs.preview_resume(PreviewWorkflowRunResumeRequest {
            run_id: run.id.clone(),
        })?;
        assert!(preview.resumable, "{preview:?}");

        runs.resume_from_failure(ResumeWorkflowRunRequest {
            run_id: run.id.clone(),
            rollback: Some(ResumeRollbackMode::Keep),
            snapshot_id: None,
        })?;
        wait_run_status(&runs, &run.id, WorkflowRunStatus::Succeeded).await?;
        let succeeded = runs.get(GetWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        let loops: Vec<_> = succeeded
            .nodes
            .iter()
            .filter(|node| node.node_id == "loop")
            .collect();
        let writers: Vec<_> = succeeded
            .nodes
            .iter()
            .filter(|node| node.node_id == "writer")
            .collect();
        // The failed round's rows (entry included) must be gone from the live view.
        let entries = succeeded
            .nodes
            .iter()
            .filter(|node| node.node_id == "entry")
            .count();
        assert_eq!(
            (loops.len(), writers.len(), entries),
            (1, 1, 1),
            "{:?}",
            succeeded.nodes
        );
        assert_ne!(loops[0].id, loop_failed_id);
        assert_eq!(loops[0].status, WorkflowNodeStatus::Succeeded);
        assert_ne!(writers[0].id, writer_failed_id);
        assert_eq!(writers[0].status, WorkflowNodeStatus::Succeeded);
        let writer_payload: serde_json::Value = serde_json::from_str(
            writers[0]
                .payload
                .as_deref()
                .ok_or("missing writer payload")?,
        )?;
        assert!(
            writer_payload
                .get("injected_failure_context")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains("Previous attempt")),
            "{writer_payload}"
        );
        assert_eq!(
            succeeded.run.output.as_deref(),
            Some(r#"{"draft":"{\"ok\":true}"}"#)
        );
        Ok(())
    })
}
