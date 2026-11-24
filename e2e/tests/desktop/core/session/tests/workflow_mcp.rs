//! Node allowlists must survive actual ACP creation, restore, rebuild, and plugin wakeups.

use super::{
    agent_ref, current_thread_runtime, drain_prompt, install_fake_opencode_plugin,
    install_mcp_plugin, open_ready_backend, seed_workspace,
};
use crate::setup::DesktopTestSetup;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_backend::{Backend, BackendError};
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Produces two independent nodes, one with a mixed allowlist and one with no MCP bindings.
fn graph() -> String {
    let agent = |id, mcps| {
        json!({"id": id, "data": {"kind": "agent", "agentConfig": {
            "executor": {"agentCli": "official/ora-space.opencode", "modelId": "anthropic/claude-sonnet-4"},
            "prompt": "hello", "interactive": true, "schemaVersion": 4, "completionSignal": "agent", "mcps": mcps
        }}})
    };
    json!({"nodes": [
        {"id": "start", "data": {"kind": "start"}},
        agent("left", json!([{"mcpId": "official/first", "enabled": true}, {"mcpId": "official/second", "enabled": false}])),
        agent("right", json!([]))
    ], "edges": [{"source": "start", "target": "left"}, {"source": "start", "target": "right"}]}).to_string()
}

/// Waits for both first turns to park, reporting node errors instead of hiding setup failures.
async fn parked_nodes(
    backend: &Backend,
    run_id: &str,
) -> Result<Vec<WorkflowNodeRun>, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let detail = backend.workflow_runs().get(GetWorkflowRunRequest {
                run_id: run_id.to_string(),
            })?;
            if let Some(error) = detail.nodes.iter().find_map(|node| node.error.as_ref()) {
                return Err(std::io::Error::other(error.clone()).into());
            }
            let nodes: Vec<_> = detail
                .nodes
                .into_iter()
                .filter(|node| node.node_type == "agent")
                .collect();
            if nodes.len() == 2
                && nodes.iter().all(|node| {
                    node.status == WorkflowNodeStatus::Pending && node.session_id.is_some()
                })
            {
                return Ok(nodes);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

/// Reads complete request observations after the corresponding response has been consumed.
fn mcp_calls(package_root: &Path) -> Result<Vec<Value>, serde_json::Error> {
    fs::read_to_string(package_root.join("mcp_calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(serde_json::from_str)
        .collect()
}

/// Sends a human continuation through the workflow-owned session's ordinary prompt boundary.
async fn follow_up(backend: &Backend, session_id: &str) -> TestResult {
    drain_prompt(backend.sessions().prompt(PromptSessionRequest {
        session_id: session_id.to_string(),
        model: None,
        record_prompt: None,
        prompt: vec![ContentBlock::Text(TextContent::new("continue"))],
    }))
    .await
}

/// The frozen choice, including an empty choice, must win over all installed plugins on every path.
#[test]
fn workflow_mcp_allowlists_survive_restore_rebuild_and_refresh() -> TestResult {
    ora_logging::with_trace_logging(|| {
        current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        let first = install_mcp_plugin(home, "first")?;
        install_mcp_plugin(home, "second")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        // Interactive completion snapshots require a committed checkout.
        for args in [vec!["init"], vec!["-c", "user.name=Test", "-c", "user.email=test@example.test", "commit", "--allow-empty", "-m", "initial"]] {
            let output = std::process::Command::new("git").args(args).current_dir(setup.root().join("workspace")).output()?;
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        }
        let workflow = backend.workflows().create(CreateWorkflowRequest { name: "MCP workflow".into(), graph: Some(graph()) })?;
        backend.workflows().publish(PublishWorkflowRequest { workflow_id: workflow.workflow.id.clone(), version: Some("v1".into()) })?;
        let run = backend.workflow_runs().create(CreateWorkflowRunRequest {
            workspace_id: workspace_id.clone(), workflow_id: workflow.workflow.id.clone(), locale: WorkflowRunLocale::EnUs,
            snapshot_id: None, kickoff_input: None, name: None, inject_last_failure: None,
        })?.run;
        backend.workflow_runs().start(StartWorkflowRunRequest { run_id: run.id.clone() })?;
        let nodes = parked_nodes(&backend, &run.id).await?;
        let left = nodes.iter().find(|node| node.node_id == "left").and_then(|node| node.session_id.as_ref()).ok_or("left session missing")?;
        let calls = mcp_calls(&package_root)?;
        assert_eq!(calls.len(), 2);
        let left_provider = calls.iter().find(|call| call["servers"] == json!(["official/first", "ora-workflow-completion"])).ok_or("left MCP selection missing")?["sessionId"].clone();
        let right_provider = calls.iter().find(|call| call["servers"] == json!(["ora-workflow-completion"])).ok_or("right MCP selection not empty")?["sessionId"].clone();

        let ordinary = backend.sessions().start(StartSessionRequest { workspace_id, agent_ref: agent_ref(), model: None }).await?;
        assert_eq!(mcp_calls(&package_root)?.last().ok_or("MCP journal is empty")?["servers"], json!(["official/first", "official/second"]));
        backend.workflows().update_draft(UpdateDraftRequest { workflow_id: workflow.workflow.id, graph: graph().replace("official/first", "official/second") })?;
        backend.sessions().stop(StopSessionRequest { session_id: left.clone() }).await?;
        follow_up(&backend, left).await?;
        parked_nodes(&backend, &run.id).await?;
        assert_eq!(mcp_calls(&package_root)?.last().ok_or("MCP journal is empty")?, &json!({"method": "session/load", "sessionId": left_provider, "servers": ["official/first", "ora-workflow-completion"]}));

        fs::write(package_root.join("refuse_session_load"), "refuse")?;
        backend.sessions().stop(StopSessionRequest { session_id: left.clone() }).await?;
        follow_up(&backend, left).await?;
        parked_nodes(&backend, &run.id).await?;
        let calls = mcp_calls(&package_root)?;
        assert_eq!(calls[calls.len()-2]["method"], "session/load");
        assert_eq!(calls.last().ok_or("MCP journal is empty")?["method"], "session/new");
        assert_eq!(calls.last().ok_or("MCP journal is empty")?["servers"], json!(["official/first", "ora-workflow-completion"]));
        let rebuilt_provider = calls.last().ok_or("MCP journal is empty")?["sessionId"].clone();
        fs::remove_file(package_root.join("refuse_session_load"))?;

        // An actual selected package revision wakes the live actor, still using its frozen IDs.
        let manifest = first.join("orax.toml");
        fs::write(&manifest, fs::read_to_string(&manifest)?.replace("1.0.0", "1.0.1"))?;
        let updated = first.with_file_name("1.0.1");
        fs::rename(&first, &updated)?;
        backend.plugins().scan(ScanPluginsRequest {}).await?;
        // The scan wakes the live actor into an MCP refresh that holds the session exclusively;
        // a prompt reaching it mid-refresh is answered with SessionBusy while the parked node
        // rolls back to Pending. Waiting for the refresh's own journal entry closes the wide
        // window, and a bounded retry settles the instant where the refresh response and the
        // queued prompt are ready together inside the actor's select.
        tokio::time::timeout(Duration::from_secs(15), async {
            while !mcp_calls(&package_root).is_ok_and(|calls| {
                calls.iter().any(|call| {
                    call["method"] == "session/load"
                        && call["sessionId"] == rebuilt_provider
                        && call["servers"] == json!(["official/first", "ora-workflow-completion"])
                })
            }) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        for attempt in 0..5 {
            match follow_up(&backend, left).await {
                Ok(()) => break,
                Err(error)
                    if attempt < 4
                        && error
                            .downcast_ref::<BackendError>()
                            .is_some_and(|error| {
                                matches!(error.public_error(), PublicError::SessionBusy(_))
                            }) =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error),
            }
        }
        parked_nodes(&backend, &run.id).await?;
        let left_provider = rebuilt_provider;
        assert!(mcp_calls(&package_root)?.iter().any(|call| call["method"] == "session/load" && call["sessionId"] == left_provider && call["servers"] == json!(["official/first", "ora-workflow-completion"])));
        assert!(mcp_calls(&package_root)?.iter().filter(|call| call["sessionId"] == right_provider).all(|call| call["servers"] == json!(["ora-workflow-completion"])));
        backend.workflow_runs().cancel(CancelWorkflowRunRequest { run_id: run.id }).await?;
        backend.sessions().stop(StopSessionRequest { session_id: ordinary.session.id }).await?;
        Ok(())
    })
    })
}

/// Polls a run until it is terminal, returning the last public detail.
async fn terminal(
    backend: &Backend,
    run_id: &str,
) -> Result<GetWorkflowRunResponse, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let detail = backend.workflow_runs().get(GetWorkflowRunRequest {
                run_id: run_id.to_string(),
            })?;
            if matches!(
                detail.run.status,
                WorkflowRunStatus::Succeeded
                    | WorkflowRunStatus::Failed
                    | WorkflowRunStatus::Cancelled
            ) {
                return Ok(detail);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

/// One agent node that binds the MCP plugin and completes on its own.
fn single_agent_graph() -> Value {
    json!({"nodes": [
        {"id": "start", "data": {"kind": "start"}},
        {"id": "first", "data": {"kind": "agent", "title": "First step", "agentConfig": {
            "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0},
            "executor": {"agentCli": "official/ora-space.opencode", "modelId": "anthropic/claude-sonnet-4"},
            "prompt": "hello", "mcps": [{"mcpId": "official/first", "enabled": true}]
        }}}
    ], "edges": [{"source": "start", "target": "first"}]})
}

/// A chain whose first agent binds the MCP plugin and whose second step fails on its first
/// prompt, so the run settles as `Failed` with a `Succeeded` predecessor.
fn failing_chain() -> Value {
    let agent = |id: &str, title: &str, prompt: &str, mcps: Value| {
        json!({"id": id, "data": {"kind": "agent", "title": title, "agentConfig": {
            "retry": {"enabled": false, "maxRetries": 0, "initialDelaySeconds": 0},
            "executor": {"agentCli": "official/ora-space.opencode", "modelId": "anthropic/claude-sonnet-4"},
            "prompt": prompt, "mcps": mcps
        }}})
    };
    json!({"nodes": [
        {"id": "start", "data": {"kind": "start"}},
        agent("first", "First step", "hello", json!([{"mcpId": "official/first", "enabled": true}])),
        agent("second", "Second step", "[fail-times:1] hello", json!([]))
    ], "edges": [{"source": "start", "target": "first"}, {"source": "first", "target": "second"}]})
}

/// Installs one MCP package that declares a required setting with no value: installed enough to
/// schedule, not configured enough to open a session.
fn install_unconfigured_mcp_plugin(home: &Path, name: &str) -> std::io::Result<PathBuf> {
    let root = home
        .join("plugins")
        .join("installed")
        .join("official")
        .join(name)
        .join("1.0.0");
    fs::create_dir_all(root.join("assets"))?;
    fs::write(
        root.join("orax.toml"),
        format!(
            "resolver = 1\nidentifier = \"{name}\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"Unconfigured MCP fixture\"\n"
        ),
    )?;
    let command = root.join("assets").join("server");
    fs::write(&command, "#!/bin/sh\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&command, fs::Permissions::from_mode(0o755))?;
    }
    fs::write(
        root.join("assets").join("config.json"),
        json!({
            "schemaVersion": 1,
            "settings": {"apiKey": {"type": "string", "title": "API key", "description": "Required credential", "required": true}},
            "transport": {"type": "stdio", "command": "assets/server", "args": [], "env": {}}
        })
        .to_string(),
    )?;
    Ok(root)
}

/// Publishes one graph and creates a pending run the way the run screen does.
fn pending_run(
    backend: &Backend,
    workspace_id: &str,
    graph: &Value,
) -> Result<WorkflowRun, Box<dyn std::error::Error>> {
    let workflow = backend.workflows().create(CreateWorkflowRequest {
        name: "MCP preflight".into(),
        graph: Some(graph.to_string()),
    })?;
    backend.workflows().publish(PublishWorkflowRequest {
        workflow_id: workflow.workflow.id.clone(),
        version: Some("v1".into()),
    })?;
    Ok(backend
        .workflow_runs()
        .create(CreateWorkflowRunRequest {
            workspace_id: workspace_id.to_string(),
            workflow_id: workflow.workflow.id,
            locale: WorkflowRunLocale::EnUs,
            snapshot_id: None,
            kickoff_input: None,
            name: None,
            inject_last_failure: None,
        })?
        .run)
}

/// Removes one installed MCP package through the plugin use case, as the plugin market does.
async fn uninstall_mcp(backend: &Backend, plugin_id: &str) -> TestResult {
    backend
        .plugins()
        .uninstall(UninstallPluginRequest {
            plugin_id: plugin_id.to_string(),
            data_disposition: PluginDataDisposition::Delete,
            hook_execution_acknowledged: false,
        })
        .await?;
    Ok(())
}

/// A finished run whose MCP disappeared cannot restart, and the refusal dispatches nothing.
#[test]
fn a_succeeded_run_cannot_restart_after_its_mcp_plugin_is_gone() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run = pending_run(&backend, &workspace_id, &single_agent_graph())?;
        backend.workflow_runs().start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        let succeeded = terminal(&backend, &run.id).await?;
        assert_eq!(
            succeeded.run.status,
            WorkflowRunStatus::Succeeded,
            "run={:?} nodes={:?}",
            succeeded.run.error,
            succeeded
                .nodes
                .iter()
                .map(|node| (&node.node_id, node.status, &node.error))
                .collect::<Vec<_>>()
        );

        uninstall_mcp(&backend, "official/first").await?;
        let journal_before = mcp_calls(&package_root)?.len();
        let sessions_before = backend
            .sessions()
            .list(ListSessionsRequest {})?
            .sessions
            .len();

        let error = backend
            .workflow_runs()
            .restart(RestartWorkflowRunRequest {
                run_id: run.id.clone(),
            })
            .err()
            .ok_or("a restart of a finished run must be refused")?;
        assert_eq!(error.public_error().code(), "workflow_mcp_not_found");
        assert_eq!(
            backend
                .workflow_runs()
                .get(GetWorkflowRunRequest {
                    run_id: run.id.clone(),
                })?
                .run,
            succeeded.run,
            "the refused restart must leave the finished run untouched"
        );
        assert_eq!(
            backend
                .sessions()
                .list(ListSessionsRequest {})?
                .sessions
                .len(),
            sessions_before,
            "the refusal must not create a Session"
        );
        assert_eq!(
            mcp_calls(&package_root)?.len(),
            journal_before,
            "the refusal must not open another ACP session"
        );
        Ok(())
    })
}

/// A failed run cannot resume around an MCP its already-succeeded node needs, because the whole
/// frozen graph stays the deployment unit.
#[test]
fn a_failed_run_cannot_resume_around_a_succeeded_nodes_mcp() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run = pending_run(&backend, &workspace_id, &failing_chain())?;
        backend.workflow_runs().start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        let failed = terminal(&backend, &run.id).await?;
        assert_eq!(failed.run.status, WorkflowRunStatus::Failed);
        assert_eq!(
            failed
                .nodes
                .iter()
                .find(|node| node.node_id == "first")
                .map(|node| node.status),
            Some(WorkflowNodeStatus::Succeeded)
        );

        uninstall_mcp(&backend, "official/first").await?;
        let journal_before = mcp_calls(&package_root)?.len();
        let error = backend
            .workflow_runs()
            .resume_from_failure(ResumeWorkflowRunRequest {
                run_id: run.id.clone(),
                rollback: None,
                snapshot_id: None,
            })
            .err()
            .ok_or("a resume must not route around a removed MCP")?;
        assert_eq!(error.public_error().code(), "workflow_mcp_not_found");
        assert_eq!(
            backend
                .workflow_runs()
                .get(GetWorkflowRunRequest {
                    run_id: run.id.clone(),
                })?
                .run,
            failed.run,
            "the refused resume must leave the failed run untouched"
        );
        assert_eq!(
            mcp_calls(&package_root)?.len(),
            journal_before,
            "the refusal must not open another ACP session"
        );
        Ok(())
    })
}

/// An installed but unconfigured MCP passes the scheduling precheck and then fails the node's
/// session setup, so one run shows both sides of the boundary.
#[test]
fn an_incomplete_mcp_passes_precheck_and_fails_the_node_session() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_unconfigured_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run = pending_run(&backend, &workspace_id, &single_agent_graph())?;

        let started = backend.workflow_runs().start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        assert_eq!(started.run.status, WorkflowRunStatus::Running);

        let failed = terminal(&backend, &run.id).await?;
        assert_eq!(failed.run.status, WorkflowRunStatus::Failed);
        let node = failed
            .nodes
            .iter()
            .find(|node| node.node_id == "first")
            .ok_or("the node row is missing")?;
        let error = node
            .error
            .as_deref()
            .ok_or("the node did not report an error")?;
        assert!(
            error.contains("configuration is incomplete"),
            "the node must fail the session setup for its incomplete configuration: {error}"
        );
        assert!(node.session_id.is_none(), "no Session may be delivered");
        assert_eq!(
            mcp_calls(&package_root)?.len(),
            0,
            "an incomplete selection must never reach ACP"
        );
        Ok(())
    })
}

/// The package can disappear between the precheck and the first dispatch: the node session setup
/// then refuses it, so nothing runs with a partial selection.
#[test]
fn uninstalling_between_precheck_and_dispatch_fails_the_node_session() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_mcp_plugin(home, "first")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let run = pending_run(&backend, &workspace_id, &single_agent_graph())?;

        let started = backend.workflow_runs().start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        assert_eq!(started.run.status, WorkflowRunStatus::Running);
        uninstall_mcp(&backend, "official/first").await?;

        let failed = terminal(&backend, &run.id).await?;
        assert_eq!(failed.run.status, WorkflowRunStatus::Failed);
        let node = failed
            .nodes
            .iter()
            .find(|node| node.node_id == "first")
            .ok_or("the node row is missing")?;
        let error = node
            .error
            .as_deref()
            .ok_or("the node did not report an error")?;
        assert!(
            error.contains("is not installed"),
            "the node must fail closed as its selected plugin disappeared: {error}"
        );
        assert!(node.session_id.is_none(), "no Session may be delivered");
        assert_eq!(
            mcp_calls(&package_root)?.len(),
            0,
            "a missing selection must never reach ACP"
        );
        Ok(())
    })
}
