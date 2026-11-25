use super::{
    TestResult, acp_calls, create_pending_run, current_thread_runtime, dirty_worktree, follow_up,
    graph, install_fake_opencode_plugin, install_mcp_plugin, mcp_calls, open_ready_backend,
    parked_nodes, publish_graph, seed_workspace, uninstall_mcp,
};
use crate::setup::DesktopTestSetup;
use ora_backend::BackendError;
use ora_contracts::{
    CancelWorkflowRunRequest, PublicError, ScanPluginsRequest, SessionMcpSetupFailedParams,
    StartWorkflowRunRequest, StopSessionRequest, UpdateDraftRequest,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

/// Restoring an existing workflow Session must resolve its whole persisted MCP selection.
#[test]
fn restoring_a_workflow_session_after_mcp_uninstall_fails_closed() -> TestResult {
    current_thread_runtime()?.block_on(async {
        let setup = DesktopTestSetup::new()?;
        let home = &setup.backend_paths().home_directory;
        let package_root = install_fake_opencode_plugin(home)?;
        install_mcp_plugin(home, "first")?;
        install_mcp_plugin(home, "second")?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        dirty_worktree(&setup)?;

        let mut graph: Value = serde_json::from_str(&graph())?;
        let selected = json!([
            {"mcpId": "official/first", "enabled": true},
            {"mcpId": "official/second", "enabled": true}
        ]);
        graph["nodes"][1]["data"]["agentConfig"]["mcps"] = selected.clone();
        let workflow_id = publish_graph(&backend, "MCP recovery", &graph)?;
        let run = create_pending_run(&backend, &workspace_id, &workflow_id)?;
        backend.workflow_runs().start(StartWorkflowRunRequest {
            run_id: run.id.clone(),
        })?;
        let nodes = parked_nodes(&backend, &run.id).await?;
        let session_id = nodes
            .iter()
            .find(|node| node.node_id == "left")
            .and_then(|node| node.session_id.clone())
            .ok_or("the workflow node must already own a Session")?;
        let original_servers = json!([
            "official/first",
            "official/second",
            "ora-workflow-completion"
        ]);
        let provider_session = mcp_calls(&package_root)?
            .into_iter()
            .find(|call| call["method"] == "session/new" && call["servers"] == original_servers)
            .ok_or("both selected MCPs must reach the original Session")?["sessionId"]
            .clone();

        backend
            .sessions()
            .stop(StopSessionRequest {
                session_id: session_id.clone(),
            })
            .await?;
        // The draft would allow a smaller selection. The persisted Session still requires both.
        graph["nodes"][1]["data"]["agentConfig"]["mcps"] = json!([selected[1]]);
        backend.workflows().update_draft(UpdateDraftRequest {
            workflow_id,
            graph: graph.to_string(),
        })?;
        uninstall_mcp(&backend, "official/first").await?;

        let before = acp_calls(&package_root);
        let error = follow_up(&backend, &session_id)
            .await
            .expect_err("an incomplete persisted selection must reject the continuation");
        let error = error
            .downcast_ref::<BackendError>()
            .ok_or(error.to_string())?;
        assert_eq!(
            error.public_error(),
            &PublicError::SessionMcpSetupFailed(Box::new(SessionMcpSetupFailedParams {
                error_code: "mcp_selected_plugin_unavailable".into(),
                plugin_id: Some("official/first".into()),
                setting_id: None,
                transport: None,
            }))
        );
        assert_eq!(acp_calls(&package_root), before);

        // Retrying with the dependency restored proves the stop/prompt route really uses load.
        install_mcp_plugin(home, "first")?;
        backend.plugins().scan(ScanPluginsRequest {}).await?;
        follow_up(&backend, &session_id).await?;
        let after = acp_calls(&package_root);
        let provider_id = provider_session
            .as_str()
            .ok_or("provider Session id missing")?;
        assert_eq!(
            &after[before.len()..],
            &[
                format!("session/load {provider_id}"),
                format!("session/prompt {provider_id}")
            ]
        );
        let calls = mcp_calls(&package_root)?;
        assert_eq!(
            calls.last(),
            Some(&json!({
                "method": "session/load",
                "sessionId": provider_session,
                "servers": original_servers,
            }))
        );
        backend
            .workflow_runs()
            .cancel(CancelWorkflowRunRequest { run_id: run.id })
            .await?;
        Ok(())
    })
}
