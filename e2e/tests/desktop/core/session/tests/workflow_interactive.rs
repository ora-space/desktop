//! Interactive-node completion through the production Backend.
//!
//! These cases drive the user-visible contract end to end: an interactive node parks, the host
//! restarts, a human turn that never settles blocks the completion, continuing the step works on
//! both provider reattachment paths, and an explicit completion advances the run. The second case
//! pins the other half of ADR D6: no message text — including "继续" — advances a node on its own.

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
use std::path::PathBuf;
use std::time::{Duration, Instant};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Deadline for one interactive round trip; the fake agent and the real backend are local.
const RUN_DEADLINE: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Marker file that makes the fake agent refuse `session/load`, as a lost provider session would.
const LOAD_REFUSAL_MARKER: &str = "refuse_session_load";
/// Prompt marker that keeps the fake agent's turn open until the host cancels it.
const HOLD_FOR_CANCEL: &str = "[hold-for-cancel]";
const AGENT_CLI: &str = "official/ora-space.opencode";

/// A graph whose only agent node awaits human input: start → agent → output.
fn interactive_graph() -> Value {
    json!({
        "nodes": [
            {"id": "start", "data": {"kind": "start"}},
            {"id": "agent", "data": {"kind": "agent", "title": "Review", "agentConfig": {
                "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0},
                "executor": {"agentCli": AGENT_CLI, "modelId": "anthropic/claude-sonnet-4"},
                "prompt": "review the plan",
                "interactive": true
            }}},
            {"id": "output", "data": {"kind": "output", "title": "Deliver"}}
        ],
        "edges": [
            {"source": "start", "target": "agent"},
            {"source": "agent", "target": "output"}
        ]
    })
}

/// Publishes the interactive graph and starts one run against it.
fn start_interactive_run(
    backend: &Backend,
    workspace_id: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let workflow = backend
        .workflows()
        .create(CreateWorkflowRequest {
            name: "Interactive".to_string(),
            graph: Some(interactive_graph().to_string()),
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

/// Reads one node row from the run projection.
fn node_of(backend: &Backend, run_id: &str, node_id: &str) -> Option<WorkflowNodeRun> {
    backend
        .workflow_runs()
        .get(GetWorkflowRunRequest {
            run_id: run_id.to_string(),
        })
        .ok()
        .and_then(|detail| {
            detail
                .nodes
                .into_iter()
                .find(|node| node.node_id == node_id)
        })
}

/// Polls the projection until the node row satisfies `ready`, yielding to the backend's tasks.
async fn wait_for_node(
    backend: &Backend,
    run_id: &str,
    node_id: &str,
    mut ready: impl FnMut(&WorkflowNodeRun) -> bool,
) -> Result<WorkflowNodeRun, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + RUN_DEADLINE;
    loop {
        if let Some(node) = node_of(backend, run_id, node_id)
            && ready(&node)
        {
            return Ok(node);
        }
        let observed = node_of(backend, run_id, node_id);
        let status = observed
            .as_ref()
            .map(|node| format!("{:?}", node.status))
            .unwrap_or_else(|| "missing".to_string());
        let error = observed.and_then(|node| node.error);
        assert!(
            Instant::now() < deadline,
            "node `{node_id}` did not reach the expected state within {RUN_DEADLINE:?} (status {status}, error {error:?})",
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Polls the projection until the run reaches a terminal status.
async fn wait_for_run_end(
    backend: &Backend,
    run_id: &str,
) -> Result<GetWorkflowRunResponse, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + RUN_DEADLINE;
    loop {
        let detail = backend.workflow_runs().get(GetWorkflowRunRequest {
            run_id: run_id.to_string(),
        })?;
        if matches!(
            detail.run.status,
            WorkflowRunStatus::Succeeded | WorkflowRunStatus::Failed
        ) {
            return Ok(detail);
        }
        assert!(
            Instant::now() < deadline,
            "run did not settle within {RUN_DEADLINE:?}: {:?}",
            detail.run.status
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Parses the parked node's completion blockers through their public contract type.
fn completion_blockers(node: &WorkflowNodeRun) -> Vec<WorkflowNodeCompletionBlocker> {
    let payload: Value =
        serde_json::from_str(node.payload.as_deref().unwrap_or("{}")).unwrap_or(Value::Null);
    serde_json::from_value(payload["completion"]["blockers"].clone()).unwrap_or_default()
}

/// Reads the parked node's `payload.completion` snapshot as the raw JSON the projection parses.
fn completion_snapshot(node: &WorkflowNodeRun) -> Value {
    let payload: Value =
        serde_json::from_str(node.payload.as_deref().unwrap_or("{}")).unwrap_or(Value::Null);
    payload["completion"].clone()
}

/// Sends one human turn on a session and drains it to its stop.
async fn say(backend: &Backend, session_id: &str, text: &str) -> TestResult {
    drain_prompt(backend.sessions().prompt(PromptSessionRequest {
        session_id: session_id.to_string(),
        prompt: vec![ContentBlock::Text(TextContent::new(text.to_string()))],
        record_prompt: None,
        model: None,
    }))
    .await
}

/// Requests a manual completion of the interactive node.
async fn complete(backend: &Backend, run_id: &str) -> Result<(), BackendError> {
    backend
        .workflow_runs()
        .complete_node(CompleteWorkflowNodeRequest {
            run_id: run_id.to_string(),
            node_id: "agent".to_string(),
            force: false,
            requester: Some(NodeCompletionRequester::Human),
        })
        .await
        .map(|_response| ())
}

/// Runs one host-restart scenario, optionally with the provider refusing `session/load`.
async fn restart_scenario(refuse_load: bool) -> TestResult {
    let setup = DesktopTestSetup::new()?;
    let package_root: PathBuf =
        install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
    let (run_id, session_id) = {
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run_id = start_interactive_run(&backend, &workspace_id)?;
        let parked = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending
        })
        .await?;
        // The first park owns its own snapshot and stays unconfirmed: the agent may have ended its
        // opening reply with a question. Assert it before any human turn starts: that turn's
        // cleanup rewrites `payload.completion`, which would hide a wrong first snapshot forever.
        let first = completion_snapshot(&parked);
        assert_eq!(
            first["stopReason"],
            json!("end_turn"),
            "the settled first turn parks as end_turn"
        );
        assert_eq!(first["provisional"], json!(false));
        assert_eq!(
            completion_blockers(&parked)
                .into_iter()
                .map(|blocker| blocker.kind)
                .collect::<Vec<_>>(),
            vec![WorkflowNodeCompletionBlockerKind::TurnUnconfirmed],
            "the opening turn parks unconfirmed instead of ready"
        );
        assert!(
            first["turnKey"]["userSeq"]
                .as_u64()
                .zip(first["turnKey"]["endedSeq"].as_u64())
                .is_some_and(|(user_seq, ended_seq)| user_seq < ended_seq),
            "the snapshot names the settled turn"
        );
        assert!(
            first["outputDigest"]
                .as_str()
                .is_some_and(|digest| digest.starts_with("sha256:")),
            "the snapshot pins the output digest"
        );
        // The opening turn cannot complete without the human's explicit confirmation.
        let rejected = complete(&backend, &run_id)
            .await
            .err()
            .ok_or("an unconfirmed opening turn cannot complete")?;
        assert_eq!(
            rejected.public_error(),
            &PublicError::WorkflowNodeIncomplete(WorkflowNodeIncompleteParams {
                node_id: "agent".to_string(),
                blockers: vec![WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::TurnUnconfirmed,
                    message: None,
                }],
            })
        );
        let session_id = parked.session_id.ok_or("the node owns a session")?;
        (run_id, session_id)
    };

    // A fresh Backend over the same data directory is what a restart leaves behind: the node row
    // and its session survive, every actor and provider session does not.
    if refuse_load {
        std::fs::write(package_root.join(LOAD_REFUSAL_MARKER), "")?;
    }
    let backend = open_ready_backend(&setup)?;
    drain_load(backend.sessions().load(LoadSessionRequest {
        session_id: session_id.clone(),
    }))
    .await?;

    // A human turn the user stops never settles as a usable output.
    let mut stream = backend
        .sessions()
        .prompt(PromptSessionRequest {
            session_id: session_id.clone(),
            prompt: vec![ContentBlock::Text(TextContent::new(format!(
                "carry on {HOLD_FOR_CANCEL}"
            )))],
            record_prompt: None,
            model: None,
        })
        .await?;
    wait_for_node(&backend, &run_id, "agent", |node| {
        node.status == WorkflowNodeStatus::Running
    })
    .await?;
    backend
        .sessions()
        .cancel_prompt(CancelSessionPromptRequest {
            session_id: session_id.clone(),
        })?;
    while let Some(event) = stream.recv().await {
        event?;
    }
    // The node returns to `Pending` from the stream's cleanup, which runs when the stream is
    // dropped, so the drop is part of the user action rather than an afterthought.
    drop(stream);

    let stopped = wait_for_node(&backend, &run_id, "agent", |node| {
        node.status == WorkflowNodeStatus::Pending && !completion_blockers(node).is_empty()
    })
    .await?;
    assert_eq!(
        completion_blockers(&stopped)
            .into_iter()
            .map(|blocker| blocker.kind)
            .collect::<Vec<_>>(),
        vec![WorkflowNodeCompletionBlockerKind::TurnStopped],
        "a cancelled turn parks as turn_stopped"
    );

    // Completing it must be refused with the blocker, not advance the run.
    let rejected = complete(&backend, &run_id)
        .await
        .err()
        .ok_or("a stopped turn cannot complete without force")?;
    assert_eq!(
        rejected.public_error(),
        &PublicError::WorkflowNodeIncomplete(WorkflowNodeIncompleteParams {
            node_id: "agent".to_string(),
            blockers: vec![WorkflowNodeCompletionBlocker {
                kind: WorkflowNodeCompletionBlockerKind::TurnStopped,
                message: None,
            }],
        })
    );
    assert_eq!(
        backend
            .workflow_runs()
            .get(GetWorkflowRunRequest {
                run_id: run_id.clone()
            })?
            .run
            .status,
        WorkflowRunStatus::AwaitingInput,
        "a refused completion leaves the run waiting for the human"
    );

    // Continuing the step is an ordinary human turn, and it carries the step boundary.
    say(&backend, &session_id, "keep the review in scope").await?;
    wait_for_node(&backend, &run_id, "agent", |node| {
        node.status == WorkflowNodeStatus::Pending && completion_blockers(node).is_empty()
    })
    .await?;
    let prompts = std::fs::read_to_string(package_root.join("acp_prompts.jsonl"))?;
    assert!(
        prompts.contains("<workflow_step_boundary>"),
        "every human turn carries the frozen step boundary"
    );

    complete(&backend, &run_id).await?;

    let finished = wait_for_run_end(&backend, &run_id).await?;
    assert_eq!(finished.run.status, WorkflowRunStatus::Succeeded);
    assert_eq!(
        finished
            .nodes
            .iter()
            .filter(|node| node.node_id == "output")
            .map(|node| node.status)
            .collect::<Vec<_>>(),
        vec![WorkflowNodeStatus::Succeeded],
        "the explicitly completed node advances the run to the next step"
    );
    Ok(())
}

/// A stopped human turn blocks completion, survives a restart, and continues on either attach path.
///
/// The two iterations cover ADR D8: `session/load` succeeding and the agent refusing it, which the
/// runtime answers by rebuilding the provider session for the next turn.
#[test]
fn a_stopped_step_blocks_completion_and_continues_after_a_restart() -> TestResult {
    current_thread_runtime()?.block_on(async {
        restart_scenario(/*refuse_load*/ false).await?;
        restart_scenario(/*refuse_load*/ true).await
    })
}

/// A plain conversation saying "继续" advances nothing: only an explicit completion moves a node.
#[test]
fn a_plain_conversation_saying_continue_does_not_advance_a_node() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run_id = start_interactive_run(&backend, &workspace_id)?;
        let parked = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending
        })
        .await?;
        let node_session = parked.session_id.ok_or("bound session")?;

        // An ordinary session is not bound to a workflow node, so its turn coordinates nothing.
        let plain = backend
            .sessions()
            .start(StartSessionRequest {
                workspace_id: workspace_id.clone(),
                agent_ref: AGENT_CLI.to_string(),
                model: None,
            })
            .await?;
        assert_ne!(plain.session.id, node_session);
        say(&backend, &plain.session.id, "继续").await?;

        let unchanged = backend.workflow_runs().get(GetWorkflowRunRequest {
            run_id: run_id.clone(),
        })?;
        assert_eq!(unchanged.run.status, WorkflowRunStatus::AwaitingInput);
        let agent = unchanged
            .nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .ok_or("agent node")?;
        assert_eq!(agent.status, WorkflowNodeStatus::Pending);
        assert!(
            unchanged.nodes.iter().all(|node| node.node_id != "output"),
            "no later node runs from a message alone"
        );

        // The same text said in the node's own session is a human turn, not an advancement.
        say(&backend, &node_session, "继续").await?;
        wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending
        })
        .await?;
        let after = backend.workflow_runs().get(GetWorkflowRunRequest {
            run_id: run_id.clone(),
        })?;
        assert_eq!(after.run.status, WorkflowRunStatus::AwaitingInput);
        assert!(
            after.nodes.iter().all(|node| node.node_id != "output"),
            "a human turn in the node's session still needs the explicit completion"
        );
        Ok(())
    })
}
