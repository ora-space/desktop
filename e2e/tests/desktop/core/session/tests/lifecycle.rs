use super::{
    agent_ref, current_thread_runtime, install_fake_opencode_plugin, main_workspace_id,
    open_ready_backend,
};
use crate::setup::DesktopTestSetup;
use agent_client_protocol_schema::v1::{ContentBlock, SessionUpdate, StopReason, TextContent};
use ora_backend::{Backend, SessionEventStream};
use ora_contracts::*;
use ora_history::{HistoryRecord, history_path, read_session_history};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;
use std::time::Duration;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Keeps all setup, actors, and interface calls under scoped TRACE and bounds stalled protocols.
fn run_case(test: impl std::future::Future<Output = TestResult>) -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?
            .block_on(async { tokio::time::timeout(Duration::from_secs(15), test).await? })
    })
}

/// Composes the production runtime over the external fake ACP process and one main Workspace.
fn fixture() -> Result<(DesktopTestSetup, Backend, String), Box<dyn std::error::Error>> {
    let setup = DesktopTestSetup::new()?;
    install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
    let backend = open_ready_backend(&setup)?;
    let workspace = setup.root().join("workspace");
    fs::create_dir_all(&workspace)?;
    backend.projects().create(CreateProjectRequest {
        name: "Lifecycle E2E".to_string(),
        main_workspace_path: workspace.to_string_lossy().into_owned(),
    })?;
    let workspace_id = main_workspace_id(&backend)?;
    Ok((setup, backend, workspace_id))
}

/// Sends the explicit fake-agent prompt that stays active until an ACP cancellation arrives.
fn held_prompt(session_id: &str) -> PromptSessionRequest {
    PromptSessionRequest {
        session_id: session_id.to_string(),
        prompt: vec![ContentBlock::Text(TextContent::new("[hold-for-cancel]"))],
        record_prompt: None,
    }
}

/// Observes the agent's first frame, proving the request is in flight before dropping or stopping.
async fn first_update(stream: &mut SessionEventStream<PromptSessionEvent>) -> TestResult {
    while let Some(event) = stream.recv().await {
        if let PromptSessionEvent::SessionUpdate {
            update: SessionUpdate::AgentMessageChunk(_),
        } = event?
        {
            return Ok(());
        }
    }
    Err("prompt ended before the held agent update".into())
}

/// Waits for a durable cancellation record, not an arbitrary delay after stream drop.
async fn cancelled_history(root: &Path, session_id: &str) -> TestResult {
    loop {
        if read_session_history(root, session_id)?
            .lines
            .iter()
            .any(|line| {
                matches!(
                    line.record,
                    HistoryRecord::TurnEnded {
                        stop_reason: StopReason::Cancelled
                    }
                )
            })
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Stream drop reaches a real actor/provider, leaves the session reusable, and later deleting an
/// active prompt waits for actor shutdown before removing the Ora row and recorded history.
#[test]
fn stream_drop_cancels_the_actor_and_delete_cleans_its_history() -> TestResult {
    run_case(async {
        let (setup, backend, workspace_id) = fixture()?;
        let sessions = backend.sessions();
        let started = sessions
            .start(StartSessionRequest {
                workspace_id,
                agent_ref: agent_ref(),
                model: None,
            })
            .await?;
        let session_id = started.session.id;
        let mut stream = sessions.prompt(held_prompt(&session_id)).await?;
        first_update(&mut stream).await?;
        drop(stream);
        let history_root = setup.backend_paths().app_data_directory.join("sessions");
        cancelled_history(&history_root, &session_id).await?;
        assert_eq!(
            sessions
                .get(GetSessionRequest {
                    session_id: session_id.clone()
                })?
                .session
                .status,
            SessionStatus::Running
        );

        // The next turn must actually reach the same live actor after cancellation settles.
        let mut stream = sessions.prompt(held_prompt(&session_id)).await?;
        first_update(&mut stream).await?;
        let mut events = backend.app_events().subscribe();
        assert_eq!(events.recv().await.transpose()?, Some(AppEvent::Ready));
        let renamed = sessions
            .rename(RenameSessionRequest {
                session_id: session_id.clone(),
                title: "User title".to_string(),
            })
            .await?;
        assert_eq!(
            sessions.get(GetSessionRequest {
                session_id: session_id.clone()
            })?,
            GetSessionResponse {
                session: renamed.session.clone()
            }
        );
        assert_eq!(renamed.session.title.as_deref(), Some("User title"));
        loop {
            let event = events
                .recv()
                .await
                .transpose()?
                .ok_or("event stream ended")?;
            if event
                == (AppEvent::SessionTitleUpdated {
                    session_id: session_id.clone(),
                })
            {
                break;
            }
        }
        let history = history_path(&history_root, &session_id)?;
        assert!(history.is_file());
        assert_eq!(
            sessions
                .delete(DeleteSessionRequest {
                    session_id: session_id.clone()
                })
                .await?,
            DeleteSessionResponse {
                session_id: session_id.clone()
            }
        );
        assert!(
            !history.exists(),
            "history is removed only after the actor stops"
        );
        assert_eq!(
            sessions
                .get(GetSessionRequest { session_id })
                .err()
                .ok_or("deleted session is still available")?
                .public_error(),
            &PublicError::SessionNotFound(EmptyErrorParams {})
        );
        drop(stream);
        Ok(())
    })
}

/// Cancelling a workflow with a live held prompt commits the run and stops its actor; bound node
/// sessions stay out of ordinary session listing and cannot be prompted after cancellation.
#[test]
fn workflow_cancellation_stops_the_live_node_session() -> TestResult {
    run_case(async {
        let (_setup, backend, workspace_id) = fixture()?;
        let run = start_interactive_workflow(&backend, workspace_id, "[hold-for-cancel]")?;
        let runs = backend.workflow_runs();
        let session_id = loop {
            let nodes = runs
                .list_node_runs(ListWorkflowNodeRunsRequest {
                    run_id: run.id.clone(),
                })?
                .nodes;
            if let Some(id) = nodes.into_iter().find_map(|node| node.session_id) {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let sessions = backend.sessions();
        let mut load = sessions
            .load(LoadSessionRequest {
                session_id: session_id.clone(),
            })
            .await?;
        loop {
            let event = load
                .recv()
                .await
                .transpose()?
                .ok_or("node session ended before prompting")?;
            if matches!(
                event,
                LoadSessionEvent::SessionUpdate {
                    update: SessionUpdate::AgentMessageChunk(_)
                }
            ) {
                break;
            }
        }
        assert_eq!(
            sessions.list(ListSessionsRequest {})?,
            ListSessionsResponse {
                sessions: Vec::new()
            }
        );
        let response = runs
            .cancel(CancelWorkflowRunRequest {
                run_id: run.id.clone(),
            })
            .await?;
        assert_eq!(response.run.status, WorkflowRunStatus::Cancelled);
        assert_eq!(
            sessions
                .get(GetSessionRequest {
                    session_id: session_id.clone()
                })?
                .session
                .status,
            SessionStatus::Stopped
        );
        let error = sessions
            .prompt(held_prompt(&session_id))
            .await
            .err()
            .ok_or("cancelled node accepted another prompt")?;
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {})
        );
        drop(load);
        Ok(())
    })
}

/// Dropping a human follow-up stream cancels the actor and restores awaiting status; manual
/// completion then uses that same coordination state and stops the session after its commit.
#[test]
fn dropping_a_workflow_follow_up_restores_awaiting_status_for_completion() -> TestResult {
    run_case(async {
        let (setup, backend, workspace_id) = fixture()?;
        let run = start_interactive_workflow(&backend, workspace_id, "Settle the first turn")?;
        let runs = backend.workflow_runs();
        let sessions = backend.sessions();
        let session_id = loop {
            let detail = runs.get(GetWorkflowRunRequest {
                run_id: run.id.clone(),
            })?;
            if detail.run.status == WorkflowRunStatus::AwaitingInput {
                break detail
                    .nodes
                    .into_iter()
                    .find_map(|node| node.session_id)
                    .ok_or("awaiting node has no session")?;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let mut stream = sessions.prompt(held_prompt(&session_id)).await?;
        first_update(&mut stream).await?;
        assert_eq!(
            runs.get(GetWorkflowRunRequest {
                run_id: run.id.clone()
            })?
            .run
            .status,
            WorkflowRunStatus::Running
        );
        drop(stream);
        let history_root = setup.backend_paths().app_data_directory.join("sessions");
        cancelled_history(&history_root, &session_id).await?;
        loop {
            if runs
                .get(GetWorkflowRunRequest {
                    run_id: run.id.clone(),
                })?
                .run
                .status
                == WorkflowRunStatus::AwaitingInput
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let completed = runs
            .complete_node(CompleteWorkflowNodeRequest {
                run_id: run.id,
                node_id: "agent".to_string(),
                requester: Some(NodeCompletionRequester::Human),
            })
            .await?;
        assert_eq!(completed.run.status, WorkflowRunStatus::Succeeded);
        assert_eq!(
            sessions
                .get(GetSessionRequest { session_id })?
                .session
                .status,
            SessionStatus::Stopped
        );
        Ok(())
    })
}

/// Creates, publishes, and starts an interactive run entirely through the public interfaces.
fn start_interactive_workflow(
    backend: &Backend,
    workspace_id: String,
    prompt: &str,
) -> Result<WorkflowRun, Box<dyn std::error::Error>> {
    let graph = serde_json::json!({"nodes":[
        {"id":"start","data":{"kind":"start"}},
        {"id":"agent","data":{"kind":"agent","agentConfig":{
            "executor":{"agentCli":agent_ref(),"modelId":"anthropic/claude-sonnet-4"},
            "interactive":true,"prompt":prompt
        }}}
    ],"edges":[{"source":"start","target":"agent"}]});
    let workflow = backend
        .workflows()
        .create(CreateWorkflowRequest {
            name: "Held workflow".to_string(),
            graph: Some(graph.to_string()),
        })?
        .workflow;
    backend.workflows().publish(PublishWorkflowRequest {
        workflow_id: workflow.id.clone(),
        version: None,
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
        })?
        .run;
    runs.start(StartWorkflowRunRequest {
        run_id: run.id.clone(),
    })?;
    Ok(run)
}
