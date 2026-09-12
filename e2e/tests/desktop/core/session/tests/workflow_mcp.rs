//! Node allowlists must survive actual ACP creation, restore, rebuild, and plugin wakeups.

use super::{
    agent_ref, current_thread_runtime, drain_prompt, install_fake_opencode_plugin,
    open_ready_backend, seed_workspace,
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

/// Installs a declarative stdio MCP; the fake agent records delivery without launching its command.
fn install_mcp(home: &Path, name: &str) -> Result<PathBuf, std::io::Error> {
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
            "resolver = 1\nidentifier = \"{name}\"\nkind = \"mcp\"\nversion = \"1.0.0\"\ndescription = \"MCP fixture\"\n"
        ),
    )?;
    let command = root.join("assets").join("server");
    fs::write(&command, "#!/bin/sh\n")?;
    // Unix discovery refuses a stdio command without an executable mode bit, and CI runs there.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&command, fs::Permissions::from_mode(0o755))?;
    }
    fs::write(root.join("assets").join("config.json"), json!({"schemaVersion": 1, "transport": {"type": "stdio", "command": "assets/server", "args": [], "env": {}}}).to_string())?;
    Ok(root)
}

/// Produces two independent nodes, one with a mixed allowlist and one with no MCP bindings.
fn graph() -> String {
    let agent = |id, mcps| {
        json!({"id": id, "data": {"kind": "agent", "agentConfig": {
            "executor": {"agentCli": "official/ora-space.opencode", "modelId": "anthropic/claude-sonnet-4"},
            "prompt": "hello", "interactive": true, "mcps": mcps
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
        let first = install_mcp(home, "first")?;
        install_mcp(home, "second")?;
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
            snapshot_id: None, kickoff_input: None, name: None,
        })?.run;
        backend.workflow_runs().start(StartWorkflowRunRequest { run_id: run.id.clone() })?;
        let nodes = parked_nodes(&backend, &run.id).await?;
        let left = nodes.iter().find(|node| node.node_id == "left").and_then(|node| node.session_id.as_ref()).ok_or("left session missing")?;
        let calls = mcp_calls(&package_root)?;
        assert_eq!(calls.len(), 2);
        let left_provider = calls.iter().find(|call| call["servers"] == json!(["official/first"])).ok_or("left MCP selection missing")?["sessionId"].clone();
        let right_provider = calls.iter().find(|call| call["servers"] == json!([])).ok_or("right MCP selection not empty")?["sessionId"].clone();

        let ordinary = backend.sessions().start(StartSessionRequest { workspace_id, agent_ref: agent_ref(), model: None }).await?;
        assert_eq!(mcp_calls(&package_root)?.last().ok_or("MCP journal is empty")?["servers"], json!(["official/first", "official/second"]));
        backend.workflows().update_draft(UpdateDraftRequest { workflow_id: workflow.workflow.id, graph: graph().replace("official/first", "official/second") })?;
        backend.sessions().stop(StopSessionRequest { session_id: left.clone() }).await?;
        follow_up(&backend, left).await?;
        parked_nodes(&backend, &run.id).await?;
        assert_eq!(mcp_calls(&package_root)?.last().ok_or("MCP journal is empty")?, &json!({"method": "session/load", "sessionId": left_provider, "servers": ["official/first"]}));

        fs::write(package_root.join("refuse_session_load"), "refuse")?;
        backend.sessions().stop(StopSessionRequest { session_id: left.clone() }).await?;
        follow_up(&backend, left).await?;
        parked_nodes(&backend, &run.id).await?;
        let calls = mcp_calls(&package_root)?;
        assert_eq!(calls[calls.len()-2]["method"], "session/load");
        assert_eq!(calls.last().ok_or("MCP journal is empty")?["method"], "session/new");
        assert_eq!(calls.last().ok_or("MCP journal is empty")?["servers"], json!(["official/first"]));
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
                        && call["servers"] == json!(["official/first"])
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
        assert!(mcp_calls(&package_root)?.iter().any(|call| call["method"] == "session/load" && call["sessionId"] == left_provider && call["servers"] == json!(["official/first"])));
        assert!(mcp_calls(&package_root)?.iter().filter(|call| call["sessionId"] == right_provider).all(|call| call["servers"] == json!([])));
        backend.workflow_runs().cancel(CancelWorkflowRunRequest { run_id: run.id }).await?;
        backend.sessions().stop(StopSessionRequest { session_id: ordinary.session.id }).await?;
        Ok(())
    })
    })
}
