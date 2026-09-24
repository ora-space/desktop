//! Automatic retry of failed agent attempts through the public workflow commands, the real
//! session driver, and the fake ACP agent.

use super::workflow_resume::run_case;
use super::{agent_ref, install_fake_opencode_plugin, main_workspace_id, open_ready_backend};
use crate::setup::DesktopTestSetup;
use ora_backend::{Backend, WorkflowRuns};
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) type TestError = Box<dyn std::error::Error>;
type TestResult = Result<(), TestError>;

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// `start → worker → output`, where `worker` has the given agent config.
pub(super) fn single_agent_graph(agent_config: Value) -> String {
    json!({
        "nodes": [
            {"id": "start", "data": {"kind": "start"}},
            {"id": "worker", "data": {"kind": "agent", "agentConfig": agent_config}},
            {"id": "output", "data": {"kind": "output"}}
        ],
        "edges": [
            {"source": "start", "target": "worker"},
            {"source": "worker", "target": "output"}
        ]
    })
    .to_string()
}

/// The fake agent, a backend that runs it, and one published workflow in a fresh project.
pub(super) struct Published {
    /// Where the fake agent writes its journals.
    pub(super) package_root: PathBuf,
    pub(super) runs: Arc<WorkflowRuns>,
    workspace_id: String,
    workflow_id: String,
    /// Held so the agent runtime lives as long as the test's runs.
    _backend: Backend,
}

impl Published {
    /// Installs the fake agent, creates a project, and publishes `graph` as its workflow.
    pub(super) fn new(setup: &DesktopTestSetup, graph: String) -> Result<Self, TestError> {
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(setup)?;
        let workspace = setup.root().join("workspace");
        fs::create_dir_all(&workspace)?;
        backend.projects().create(CreateProjectRequest {
            name: "Retry E2E".to_string(),
            main_workspace_path: workspace.to_string_lossy().into_owned(),
        })?;
        let workspace_id = main_workspace_id(&backend)?;
        let workflow = backend
            .workflows()
            .create(CreateWorkflowRequest {
                name: "Retry".to_string(),
                graph: Some(graph),
            })?
            .workflow;
        backend.workflows().publish(PublishWorkflowRequest {
            workflow_id: workflow.id.clone(),
            version: Some("v1".to_string()),
        })?;
        Ok(Self {
            package_root,
            runs: backend.workflow_runs(),
            workspace_id,
            workflow_id: workflow.id,
            _backend: backend,
        })
    }

    /// Creates and starts one run of the workflow; returns its id.
    pub(super) fn start_run(&self, inject_last_failure: Option<bool>) -> Result<String, TestError> {
        let run = self
            .runs
            .create(CreateWorkflowRunRequest {
                workspace_id: self.workspace_id.clone(),
                workflow_id: self.workflow_id.clone(),
                locale: WorkflowRunLocale::EnUs,
                snapshot_id: None,
                kickoff_input: None,
                name: None,
                inject_last_failure,
            })?
            .run;
        self.runs.start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        Ok(run.id)
    }

    /// Reads the run through `get_workflow_run`.
    pub(super) fn detail(&self, run_id: &str) -> Result<GetWorkflowRunResponse, TestError> {
        Ok(self.runs.get(GetWorkflowRunRequest {
            run_id: run_id.to_string(),
        })?)
    }

    /// Waits for the run to end, asserts it ended as `expected`, and returns its detail.
    ///
    /// Waiting for any end rather than for `expected` makes a run that ends the other way fail
    /// at once with its nodes shown, instead of at the timeout.
    pub(super) async fn settle(
        &self,
        run_id: &str,
        expected: WorkflowRunStatus,
    ) -> Result<GetWorkflowRunResponse, TestError> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let detail = self.detail(run_id)?;
            match detail.run.status {
                WorkflowRunStatus::Succeeded
                | WorkflowRunStatus::Failed
                | WorkflowRunStatus::Cancelled => {
                    assert_eq!(detail.run.status, expected, "{:#?}", detail.nodes);
                    return Ok(detail);
                }
                WorkflowRunStatus::Pending
                | WorkflowRunStatus::Running
                | WorkflowRunStatus::AwaitingInput => {}
            }
            if Instant::now() >= deadline {
                return Err(format!("run {run_id} did not end; expected {expected:?}").into());
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

/// Starts a run of the published workflow and waits for it to succeed; returns its detail.
async fn run_to_success(published: &Published) -> Result<GetWorkflowRunResponse, TestError> {
    let run_id = published.start_run(None)?;
    published
        .settle(&run_id, WorkflowRunStatus::Succeeded)
        .await
}

/// Polls the run until its `worker` attempt waits for an automatic retry; returns the detail
/// read at that moment and the attempt's persisted `retry_wait`.
pub(super) async fn wait_for_retry(
    published: &Published,
    run_id: &str,
) -> Result<(GetWorkflowRunResponse, Value), TestError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let detail = published.detail(run_id)?;
        let wait = detail
            .nodes
            .iter()
            .find(|node| node.node_id == "worker")
            .and_then(|node| node.payload.as_deref())
            .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
            .and_then(|payload| payload.get("retry_wait").cloned());
        if let Some(wait) = wait {
            return Ok((detail, wait));
        }
        if Instant::now() >= deadline {
            return Err(format!("the worker of run {run_id} never waited for a retry").into());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// The sessions the fake agent served a `session/prompt` for, in order.
pub(super) fn prompted_sessions(package_root: &Path) -> Vec<String> {
    fs::read_to_string(package_root.join("acp_calls.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.strip_prefix("session/prompt "))
        .map(str::to_string)
        .collect()
}

/// The text of every prompt the fake agent received, in order.
pub(super) fn prompt_texts(package_root: &Path) -> Result<Vec<String>, TestError> {
    fs::read_to_string(package_root.join("prompts.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let entry: Value = serde_json::from_str(line)?;
            Ok(entry["prompt"]
                .as_str()
                .ok_or("prompt journal entry without text")?
                .to_string())
        })
        .collect()
}

/// The markers of the prompts the fake agent failed on purpose, one per line.
pub(super) fn prompt_failures(package_root: &Path) -> String {
    fs::read_to_string(package_root.join("prompt_failures.txt")).unwrap_or_default()
}

/// The single live `worker` row and its parsed payload.
pub(super) fn worker(
    detail: &GetWorkflowRunResponse,
) -> Result<(WorkflowNodeRun, Value), TestError> {
    let workers: Vec<_> = detail
        .nodes
        .iter()
        .filter(|node| node.node_id == "worker")
        .collect();
    assert_eq!(workers.len(), 1, "{:?}", detail.nodes);
    let payload = serde_json::from_str(
        workers[0]
            .payload
            .as_deref()
            .ok_or("missing worker payload")?,
    )?;
    Ok((workers[0].clone(), payload))
}

/// The payload of the single live `worker` row, which must have succeeded.
fn worker_payload(detail: &GetWorkflowRunResponse) -> Result<Value, TestError> {
    let (row, payload) = worker(detail)?;
    assert_eq!(row.status, WorkflowNodeStatus::Succeeded);
    Ok(payload)
}

/// `(node_id, attempt, kind)` of every attempt in the failed-attempt history, oldest first.
pub(super) fn attempts(detail: &GetWorkflowRunResponse) -> Vec<(String, u32, String)> {
    detail
        .failed_attempts
        .iter()
        .flatten()
        .map(|attempt| {
            (
                attempt.node_id.clone(),
                attempt.attempt,
                attempt.kind.clone(),
            )
        })
        .collect()
}

/// E3: the agent's first prompt fails; the node retries by itself in a fresh session with no
/// failure block, the run succeeds without a resume, and the failed attempt stays readable.
#[test]
fn a_failed_session_is_retried_automatically_and_the_run_succeeds() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let published = Published::new(
            &setup,
            single_agent_graph(json!({
                "executor": {"agentCli": agent_ref(), "modelId": "anthropic/claude-sonnet-4"},
                "prompt": "[fail-times:1] build it",
                "retry": {"enabled": true, "maxRetries": 2, "initialDelaySeconds": 0}
            })),
        )?;
        let package_root = &published.package_root;
        let detail = run_to_success(&published).await?;

        let failed = detail.failed_attempts.clone().unwrap_or_default();
        assert_eq!(
            failed
                .iter()
                .map(|attempt| (
                    attempt.node_id.as_str(),
                    attempt.attempt,
                    attempt.kind.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![("worker", 1, "session")]
        );
        // The top-level message of a session failure is generic; the agent's own error text
        // reaches the attempt history through the source chain.
        assert!(
            failed[0]
                .source_chain
                .iter()
                .any(|line| line.contains("fake agent failed prompt 1 of 1 on purpose")),
            "{:?}",
            failed[0]
        );
        // Exactly the one planned prompt failure happened; the retry's prompt went through.
        assert_eq!(prompt_failures(package_root), "[fail-times:1]\n");
        let payload = worker_payload(&detail)?;
        assert_eq!(
            (
                &payload["auto_retry"],
                payload.get("injected_failure_context")
            ),
            (&json!({"retry": 1, "max_retries": 2}), None),
            "{payload}"
        );
        let sessions = prompted_sessions(package_root);
        assert_eq!(sessions.len(), 2, "{sessions:?}");
        assert_ne!(sessions[0], sessions[1]);
        // Ora's own session ids: the failed attempt keeps its session, the retry has another.
        let live_session = detail
            .nodes
            .iter()
            .find(|node| node.node_id == "worker")
            .and_then(|node| node.session_id.clone());
        assert!(failed[0].session_id.is_some() && live_session.is_some());
        assert_ne!(failed[0].session_id, live_session);
        Ok(())
    })
}

/// E4: a structured-output failure is retried with the failure block injected once, which the
/// fake agent answers with valid JSON.
#[test]
fn a_structured_output_failure_is_retried_with_the_failure_injected() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let published = Published::new(
            &setup,
            single_agent_graph(json!({
                "executor": {"agentCli": agent_ref(), "modelId": "anthropic/claude-sonnet-4"},
                "prompt": "answer in JSON",
                "retry": {"enabled": true, "maxRetries": 1, "initialDelaySeconds": 0},
                "outputContract": {
                    "type": "structured",
                    "textExposure": "includeFinalText",
                    "schema": {
                        "type": "object",
                        "properties": {"ok": {"type": "boolean"}},
                        "required": ["ok"]
                    }
                }
            })),
        )?;
        let detail = run_to_success(&published).await?;

        assert_eq!(
            attempts(&detail),
            vec![("worker".to_string(), 1, "structured_output".to_string())]
        );
        let payload = worker_payload(&detail)?;
        let injected = payload["injected_failure_context"]
            .as_str()
            .ok_or("the retry must record the injected failure block")?;
        assert_eq!(
            injected.matches("Previous attempt").count(),
            1,
            "{injected}"
        );
        assert_eq!(prompted_sessions(&published.package_root).len(), 2);
        Ok(())
    })
}
