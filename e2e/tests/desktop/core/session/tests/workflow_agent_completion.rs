//! Agent completion signals through the production Backend and the host MCP endpoint.
//!
//! The fake agent calls the real loopback completion tool from inside its prompt turn, so these
//! cases cover the wire path the unit tests only model: token issuance, the MCP round trip, the
//! attempt-bound park, the confirmation gate, and the server-derived completion payload.

use super::workflow_interactive::{wait_for_node, wait_for_run_end};
use super::{
    current_thread_runtime, drain_load, drain_prompt, install_fake_opencode_plugin,
    open_ready_backend, seed_workspace,
};
use crate::setup::DesktopTestSetup;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_backend::{Backend, BackendError};
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::io;
use std::path::Path;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const AGENT_CLI: &str = "official/ora-space.opencode";
const SKIP_COMPLETION: &str = "[fake-agent-skip-completion]";

/// Builds an interactive agent node with the requested completion policy and one output step.
fn completion_graph(prompt: &str, completion_signal: &str, auto_accept: bool) -> Value {
    json!({
        "nodes": [
            {"id": "start", "data": {"kind": "start"}},
            {"id": "agent", "data": {"kind": "agent", "title": "Review", "agentConfig": {
                "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0},
                "executor": {"agentCli": AGENT_CLI, "modelId": "anthropic/claude-sonnet-4"},
                "prompt": prompt,
                "interactive": true,
                "completionSignal": completion_signal,
                "autoAcceptAgentCompletion": auto_accept
            }}},
            {"id": "output", "data": {"kind": "output", "title": "Deliver"}}
        ],
        "edges": [
            {"source": "start", "target": "agent"},
            {"source": "agent", "target": "output"}
        ]
    })
}

/// Publishes the graph and starts one run against it.
fn start_run(
    backend: &Backend,
    workspace_id: &str,
    prompt: &str,
    completion_signal: &str,
    auto_accept: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let workflow = backend
        .workflows()
        .create(CreateWorkflowRequest {
            name: "Agent completion".to_string(),
            graph: Some(completion_graph(prompt, completion_signal, auto_accept).to_string()),
        })?
        .workflow;
    backend.workflows().publish(PublishWorkflowRequest {
        workflow_id: workflow.id.clone(),
        version: Some("v1".to_string()),
    })?;
    let run = backend
        .workflow_runs()
        .create(CreateWorkflowRunRequest {
            inject_last_failure: None,
            workspace_id: workspace_id.to_string(),
            workflow_id: workflow.id,
            locale: WorkflowRunLocale::EnUs,
            snapshot_id: None,
            kickoff_input: None,
            name: None,
        })?
        .run;
    backend
        .workflow_runs()
        .update_input(UpdateWorkflowRunInputRequest {
            run_id: run.id.clone(),
            input: None,
            variables: Default::default(),
        })?;
    backend.workflow_runs().start(StartWorkflowRunRequest {
        run_id: run.id.clone(),
    })?;
    Ok(run.id)
}

/// Reads the parked node's payload as the production projection stores it.
fn payload(node: &WorkflowNodeRun) -> Value {
    serde_json::from_str(node.payload.as_deref().unwrap_or("{}")).unwrap_or(Value::Null)
}

/// Lists the snapshot blocker kinds in the order the backend persisted them.
fn blocker_kinds(payload: &Value) -> Vec<&str> {
    payload["completion"]["blockers"]
        .as_array()
        .map(|blockers| {
            blockers
                .iter()
                .filter_map(|blocker| blocker["kind"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

/// Reads the host completion tool calls the fake agent journaled.
fn completion_calls(package_root: &Path) -> Result<Vec<Value>, serde_json::Error> {
    std::fs::read_to_string(package_root.join("mcp_completion_calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(serde_json::from_str)
        .collect()
}

/// Reads every ACP session boundary the fake agent served with its delivered server list.
fn mcp_calls(package_root: &Path) -> Result<Vec<Value>, serde_json::Error> {
    std::fs::read_to_string(package_root.join("mcp_calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(serde_json::from_str)
        .collect()
}

/// Creates the project checkout these cases run against, committed for the node baseline.
fn seeded_workspace(
    setup: &DesktopTestSetup,
    backend: &Backend,
) -> Result<String, Box<dyn std::error::Error>> {
    let workspace_id = seed_workspace(setup, backend)?;
    let workspace = setup.root().join("workspace");
    for args in [
        vec!["init"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    ] {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&workspace)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(String::from_utf8_lossy(&output.stderr)).into());
        }
    }
    Ok(workspace_id)
}

/// Sends one human continuation through the workflow-owned session.
async fn say(backend: &Backend, session_id: &str, text: &str) -> TestResult {
    drain_prompt(backend.sessions().prompt(PromptSessionRequest {
        session_id: session_id.to_string(),
        prompt: vec![ContentBlock::Text(TextContent::new(text))],
        record_prompt: None,
        model: None,
    }))
    .await
}

/// Requests a completion with the approvals the case is testing.
async fn complete(
    backend: &Backend,
    run_id: &str,
    confirm_initial_turn: Option<bool>,
    force: bool,
    confirm_agent_completion: Option<ConfirmAgentCompletion>,
) -> Result<(), BackendError> {
    backend
        .workflow_runs()
        .complete_node(CompleteWorkflowNodeRequest {
            run_id: run_id.to_string(),
            node_id: "agent".to_string(),
            force,
            confirm_initial_turn,
            confirm_agent_completion,
            requester: None,
        })
        .await
        .map(|_response| ())
}

/// The agent calls the host tool, the park binds the signal, and the human confirmation completes.
#[test]
fn the_agent_signal_binds_to_the_turn_and_the_human_confirmation_completes_it() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seeded_workspace(&setup, &backend)?;
        let run_id = start_run(&backend, &workspace_id, "review the plan", "agent", false)?;

        let parked = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending && node.session_id.is_some()
        })
        .await?;

        // The agent actually called the loopback tool and the host accepted this attempt's token.
        let calls = completion_calls(&package_root)?;
        assert_eq!(
            calls.len(),
            1,
            "one prompt attempt makes one tool call: {calls:?}"
        );
        assert_eq!(calls[0]["accepted"], json!(true), "{calls:?}");
        let signal_id = calls[0]["signalId"]
            .as_str()
            .filter(|signal_id| !signal_id.is_empty())
            .ok_or("the accepted call reports its signal id")?
            .to_string();

        let parked_payload = payload(&parked);
        assert_eq!(parked_payload["completionPolicy"], json!("agent"));
        assert_eq!(parked_payload["agentSignal"]["signalId"], json!(signal_id));
        assert_eq!(
            parked_payload["agentSignal"]["turnKey"], parked_payload["completion"]["turnKey"],
            "the signal is bound to the turn the attempt settled"
        );
        assert!(
            parked_payload.get("confirmation").is_none(),
            "parking records the intent, not a confirmation"
        );
        assert_eq!(
            parked_payload["completion"]["turnOrigin"],
            json!("workflow")
        );
        assert_eq!(
            blocker_kinds(&parked_payload),
            vec!["turn_unconfirmed", "agent_confirmation_pending"],
            "the park materializes the signal gate the card and the completion both read"
        );
        assert_eq!(
            parked_payload["completion"]["parkKind"],
            json!("awaiting_user")
        );

        // A plain completion cannot cross the signal gate, and the first turn is still unconfirmed.
        let rejected = complete(&backend, &run_id, None, false, None)
            .await
            .err()
            .ok_or("a signaled turn cannot complete without confirming the signal")?;
        assert_eq!(
            rejected.public_error(),
            &PublicError::WorkflowNodeIncomplete(WorkflowNodeIncompleteParams {
                node_id: "agent".to_string(),
                blockers: vec![
                    WorkflowNodeCompletionBlocker {
                        kind: WorkflowNodeCompletionBlockerKind::TurnUnconfirmed,
                        message: None,
                    },
                    WorkflowNodeCompletionBlocker {
                        kind: WorkflowNodeCompletionBlockerKind::AgentConfirmationPending,
                        message: None,
                    },
                ],
            })
        );

        // A stale id is rejected instead of confirming whatever signal is current.
        let stale = complete(
            &backend,
            &run_id,
            Some(true),
            false,
            Some(ConfirmAgentCompletion {
                signal_id: "stale-signal".to_string(),
            }),
        )
        .await
        .err()
        .ok_or("a stale signal id cannot confirm the parked turn")?;
        assert!(
            matches!(
                stale.public_error(),
                PublicError::WorkflowNodeSignalChanged(params)
                    if params.expected.as_deref() == Some(signal_id.as_str())
            ),
            "unexpected rejection: {stale:?}"
        );

        complete(
            &backend,
            &run_id,
            Some(true),
            false,
            Some(ConfirmAgentCompletion {
                signal_id: signal_id.clone(),
            }),
        )
        .await?;

        let finished = wait_for_run_end(&backend, &run_id).await?;
        assert_eq!(finished.run.status, WorkflowRunStatus::Succeeded);
        let agent = finished
            .nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .ok_or("agent node")?;
        assert_eq!(agent.status, WorkflowNodeStatus::Succeeded);
        let completed = payload(agent);
        assert_eq!(
            completed["requester"],
            json!("agent"),
            "the requester is derived from the accepted signal, not from the client"
        );
        assert_eq!(completed["confirmation"]["by"], json!("human"));
        assert!(completed["confirmation"]["at"].as_i64().is_some());
        assert!(
            completed.get("completion_forced").is_none(),
            "a confirmation is an ordinary completion, never a forced one"
        );
        Ok(())
    })
}

/// The first-turn no-signal exception completes only with both the opening confirmation and force.
#[test]
fn the_missing_signal_first_turn_uses_the_documented_force_exception() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seeded_workspace(&setup, &backend)?;
        let run_id = start_run(
            &backend,
            &workspace_id,
            &format!("review the plan {SKIP_COMPLETION}"),
            "agent",
            false,
        )?;

        let parked = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending && node.session_id.is_some()
        })
        .await?;
        assert!(
            completion_calls(&package_root)?.is_empty(),
            "the marked turn must not signal completion"
        );
        assert!(payload(&parked).get("agentSignal").is_none());
        assert_eq!(
            blocker_kinds(&payload(&parked)),
            vec!["turn_unconfirmed", "agent_signal_pending"]
        );
        assert_eq!(
            payload(&parked)["completion"]["parkKind"],
            json!("awaiting_user")
        );

        let plain = complete(&backend, &run_id, None, false, None)
            .await
            .err()
            .ok_or("a missing signal cannot complete")?;
        assert!(
            matches!(
                plain.public_error(),
                PublicError::WorkflowNodeIncomplete(WorkflowNodeIncompleteParams { blockers, .. })
                    if blockers.iter().map(|blocker| blocker.kind).collect::<Vec<_>>()
                        == vec![
                            WorkflowNodeCompletionBlockerKind::TurnUnconfirmed,
                            WorkflowNodeCompletionBlockerKind::AgentSignalPending,
                        ]
            ),
            "unexpected rejection: {plain:?}"
        );

        // Force alone cannot lift the opening turn's own confirmation; the signal stays pending too.
        let forced_only = complete(&backend, &run_id, None, true, None)
            .await
            .err()
            .ok_or("force alone cannot complete an unconfirmed opening turn")?;
        assert!(
            matches!(
                forced_only.public_error(),
                PublicError::WorkflowNodeIncomplete(WorkflowNodeIncompleteParams { blockers, .. })
                    if blockers.iter().map(|blocker| blocker.kind).collect::<Vec<_>>()
                        == vec![
                            WorkflowNodeCompletionBlockerKind::TurnUnconfirmed,
                            WorkflowNodeCompletionBlockerKind::AgentSignalPending,
                        ]
            ),
            "unexpected rejection: {forced_only:?}"
        );

        complete(&backend, &run_id, Some(true), true, None).await?;

        let finished = wait_for_run_end(&backend, &run_id).await?;
        assert_eq!(finished.run.status, WorkflowRunStatus::Succeeded);
        let agent = finished
            .nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .ok_or("agent node")?;
        let completed = payload(agent);
        assert_eq!(completed["requester"], json!("human"));
        assert!(completed.get("confirmation").is_none());
        assert_eq!(
            completed["completion_forced"]["blockers"],
            json!([{ "kind": "agent_signal_pending" }]),
            "the audit records the signal blocker force actually covered"
        );
        assert_eq!(completed["completion_forced"]["stop_cause"], Value::Null);
        Ok(())
    })
}

/// Auto-accept waits for a settled human turn and records the automatic attribution.
#[test]
fn auto_accept_completes_a_settled_human_turn_without_the_card() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seeded_workspace(&setup, &backend)?;
        let run_id = start_run(
            &backend,
            &workspace_id,
            &format!("review the plan {SKIP_COMPLETION}"),
            "agent",
            true,
        )?;

        // The opening turn never auto-accepts, even with the node opted in.
        let parked = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending && node.session_id.is_some()
        })
        .await?;
        let session_id = parked.session_id.clone().ok_or("the node owns a session")?;
        assert!(
            completion_calls(&package_root)?.is_empty(),
            "the opening turn is not a settled human turn"
        );
        assert_eq!(
            backend
                .workflow_runs()
                .get(GetWorkflowRunRequest {
                    run_id: run_id.clone()
                })?
                .run
                .status,
            WorkflowRunStatus::AwaitingInput
        );

        say(&backend, &session_id, "carry on").await?;

        let finished = wait_for_run_end(&backend, &run_id).await?;
        assert_eq!(finished.run.status, WorkflowRunStatus::Succeeded);
        let agent = finished
            .nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .ok_or("agent node")?;
        let completed = payload(agent);
        assert_eq!(completed["completionPolicy"], json!("agent"));
        assert_eq!(completed["autoAcceptAgentCompletion"], json!(true));
        assert_eq!(completed["requester"], json!("agent"));
        assert_eq!(completed["confirmation"]["by"], json!("auto"));
        assert!(
            completed.get("completion_forced").is_none(),
            "auto-accept is an ordinary completion"
        );
        let calls = completion_calls(&package_root)?;
        assert_eq!(calls.len(), 1, "only the human turn signaled: {calls:?}");
        assert_eq!(calls[0]["accepted"], json!(true));
        Ok(())
    })
}

/// A legacy `end_turn` node must not receive the host completion tool at all.
#[test]
fn a_legacy_end_turn_node_never_receives_the_completion_tool() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seeded_workspace(&setup, &backend)?;
        let run_id = start_run(
            &backend,
            &workspace_id,
            "review the plan",
            "end_turn",
            false,
        )?;

        let parked = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending && node.session_id.is_some()
        })
        .await?;

        let session_news: Vec<_> = mcp_calls(&package_root)?
            .into_iter()
            .filter(|call| call["method"] == "session/new")
            .collect();
        assert_eq!(
            session_news.len(),
            1,
            "one node session opens: {session_news:?}"
        );
        assert_eq!(
            session_news[0]["servers"],
            json!([]),
            "a legacy node must not see the host completion capability"
        );
        assert!(
            completion_calls(&package_root)?.is_empty(),
            "no attempt token is issued, so the fake agent cannot signal"
        );
        assert_eq!(blocker_kinds(&payload(&parked)), vec!["turn_unconfirmed"]);
        Ok(())
    })
}

/// A new turn without a signal must clear the previous turn's signal and confirmation.
#[test]
fn a_new_turn_without_a_signal_clears_the_previous_one() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seeded_workspace(&setup, &backend)?;
        let run_id = start_run(&backend, &workspace_id, "review the plan", "agent", false)?;

        let first = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending
                && node.payload.as_deref().is_some_and(|payload| {
                    serde_json::from_str::<Value>(payload)
                        .is_ok_and(|payload| payload["agentSignal"]["signalId"].is_string())
                })
        })
        .await?;
        let session_id = first.session_id.clone().ok_or("the node owns a session")?;
        let first_signal = payload(&first)["agentSignal"]["signalId"]
            .as_str()
            .ok_or("first signal")?
            .to_string();

        // The human turn carries the skip marker, so this attempt produces no signal of its own.
        say(
            &backend,
            &session_id,
            &format!("carry on {SKIP_COMPLETION}"),
        )
        .await?;

        let second = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending
                && node.payload.as_deref().is_some_and(|payload| {
                    serde_json::from_str::<Value>(payload)
                        .is_ok_and(|payload| payload["agentSignal"].is_null())
                })
        })
        .await?;
        let second_payload = payload(&second);
        assert_eq!(second_payload["agentSignal"], Value::Null);
        assert_eq!(second_payload["confirmation"], Value::Null);
        assert_eq!(
            blocker_kinds(&second_payload),
            vec!["agent_signal_pending"],
            "the settled human turn waits for a fresh signal"
        );
        assert_ne!(
            second_payload["agentSignal"]["signalId"],
            json!(first_signal),
            "the previous turn's signal must not survive"
        );

        // Forcing this turn is a human completion, not the stale agent attribution.
        complete(&backend, &run_id, None, true, None).await?;
        let finished = wait_for_run_end(&backend, &run_id).await?;
        assert_eq!(finished.run.status, WorkflowRunStatus::Succeeded);
        let agent = finished
            .nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .ok_or("agent node")?;
        let completed = payload(agent);
        assert_eq!(completed["requester"], json!("human"));
        assert!(completed.get("confirmation").is_none());
        assert_eq!(completed["completion_forced"]["by"], json!("human"));
        Ok(())
    })
}

/// A restarted Backend must restore the completion tool before the next workflow turn.
#[test]
fn a_restarted_backend_restores_the_completion_tool() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let (run_id, session_id) = {
            let backend = open_ready_backend(&setup)?;
            let workspace_id = seeded_workspace(&setup, &backend)?;
            let run_id = start_run(&backend, &workspace_id, "review the plan", "agent", false)?;
            let parked = wait_for_node(&backend, &run_id, "agent", |node| {
                node.status == WorkflowNodeStatus::Pending
                    && node.payload.as_deref().is_some_and(|payload| {
                        serde_json::from_str::<Value>(payload)
                            .is_ok_and(|payload| payload["agentSignal"]["signalId"].is_string())
                    })
            })
            .await?;
            let session_id = parked.session_id.ok_or("the node owns a session")?;
            (run_id, session_id)
        };

        // A fresh Backend over the same data directory is what a restart leaves behind.
        let backend = open_ready_backend(&setup)?;
        drain_load(backend.sessions().load(LoadSessionRequest {
            session_id: session_id.clone(),
        }))
        .await?;

        say(&backend, &session_id, "carry on").await?;
        let parked = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending
                && node.payload.as_deref().is_some_and(|payload| {
                    serde_json::from_str::<Value>(payload)
                        .is_ok_and(|payload| payload["agentSignal"]["signalId"].is_string())
                })
        })
        .await?;
        assert!(
            mcp_calls(&package_root)?
                .iter()
                .any(|call| call["method"] == "session/load"
                    && call["sessionId"].is_string()
                    && call["servers"].as_array().is_some_and(|servers| servers
                        .iter()
                        .any(|server| server == "ora-workflow-completion"))),
            "the restored session must keep the host completion server"
        );
        let signal_id = payload(&parked)["agentSignal"]["signalId"]
            .as_str()
            .ok_or("restored turn signal")?
            .to_string();
        assert_eq!(
            completion_calls(&package_root)?.len(),
            2,
            "the fake agent called the host tool once per turn"
        );

        complete(
            &backend,
            &run_id,
            None,
            false,
            Some(ConfirmAgentCompletion { signal_id }),
        )
        .await?;
        let finished = wait_for_run_end(&backend, &run_id).await?;
        assert_eq!(finished.run.status, WorkflowRunStatus::Succeeded);
        Ok(())
    })
}
