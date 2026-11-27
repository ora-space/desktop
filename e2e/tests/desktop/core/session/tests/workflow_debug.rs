//! Run-page debug commands through the real scheduler, SQLite store and fake ACP process.
use super::agent_ref;
use super::workflow_resume::run_case;
use super::workflow_retry::{Published, TestError, prompt_texts, single_agent_graph, worker};
use crate::setup::DesktopTestSetup;
use ora_contracts::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

/// Starting from the visible attempt must change only the new dispatch, even after a retry.
#[test]
fn editing_a_finished_worker_dispatches_the_new_prompt_once() -> Result<(), TestError> {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let graph = single_agent_graph(json!({
            "executor": {"agentCli": agent_ref(), "modelId": "anthropic/claude-sonnet-4"},
            "prompt": "first debug prompt"
        }));
        let published = Published::new(&setup, graph)?;
        let run = published.start_run(None)?;
        let detail = published.settle(&run, WorkflowRunStatus::Succeeded).await?;
        let (old, _) = worker(&detail)?;
        let selection = WorkflowDebugSelection::NodeRunId(old.id.clone());
        let context = published
            .runs
            .debug_context(GetWorkflowDebugContextRequest {
                run_id: run.clone(),
                selection: selection.clone(),
            })?;
        assert_eq!(
            Some(&context.target.scope_id),
            detail.root_scope_id.as_ref()
        );
        let effective: Value = serde_json::from_str(&context.graph)?;
        let node = effective["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == "worker")
            .unwrap();
        assert_eq!(node["data"]["agentConfig"]["prompt"], "first debug prompt");
        let preview = published.runs.debug_preview(PreviewWorkflowDebugRequest {
            run_id: run.clone(),
            selection,
            base_revision: context.base_revision,
            patch: vec![WorkflowDebugPatch::Replace {
                path: "/agentConfig/prompt".into(),
                value: json!("second debug prompt"),
            }],
        })?;
        assert!(preview.impact.invalidate.contains(&old.id));
        let request = CommitWorkflowDebugRequest {
            run_id: run.clone(),
            preview_id: preview.preview_id,
            request_id: "confirm-once".into(),
        };
        let receipt = published.runs.debug_commit(request.clone()).await?;
        assert_eq!(receipt.phase, WorkflowDebugPhase::Applied { revision: 1 });
        assert_eq!(published.runs.debug_commit(request).await?, receipt);
        let detail = published.settle(&run, WorkflowRunStatus::Succeeded).await?;
        let (new, _) = worker(&detail)?;
        assert_ne!(old.id, new.id);
        let prompts = prompt_texts(&published.package_root)?;
        assert_eq!(prompts.len(), 2);
        assert!(prompts[1].contains("second debug prompt"));
        let attempt = |id| {
            published
                .runs
                .debug_attempt(GetWorkflowDebugAttemptRequest {
                    run_id: run.clone(),
                    node_run_id: id,
                })
        };
        assert!(attempt(old.id)?.superseded);
        assert_eq!(attempt(new.id)?.revision, Some(1));
        let history = published
            .runs
            .debug_history(GetWorkflowDebugHistoryRequest { run_id: run })?;
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].operation, receipt);
        Ok(())
    })
}
