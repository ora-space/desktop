//! Automatic Agent nodes keep the ordinary Git-visible completion diff.

use super::payload::report_outcome;
use super::{AgentNodeOutcome, capture_agent_baseline};
use crate::workflow::run::worktree::{
    BaselineScope, capture_worktree_snapshot, compute_file_changes,
};
use agent_client_protocol_schema::v1::StopReason;
use ora_application::{FileChange, NodeFailure, WorkflowGraph, WorkflowRunCallback};
use ora_domain::{WorkflowNodeRunId, WorkflowRunId};
use pretty_assertions::assert_eq;
use std::sync::{Arc, Mutex};

/// Records the file changes the automatic completion callback receives.
#[derive(Default)]
struct RecordingCallback {
    completions: Mutex<Vec<Vec<FileChange>>>,
}

impl WorkflowRunCallback for RecordingCallback {
    fn complete_node(
        &self,
        _run_id: &WorkflowRunId,
        _node_run_id: &WorkflowNodeRunId,
        _output: Option<String>,
        _structured_output: Option<serde_json::Value>,
        _stop_reason: Option<String>,
        file_changes: Vec<FileChange>,
    ) {
        self.completions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(file_changes);
    }

    fn fail_node(
        &self,
        _run_id: &WorkflowRunId,
        _node_run_id: &WorkflowNodeRunId,
        failure: NodeFailure,
    ) {
        panic!("automatic completion must not fail: {failure:?}");
    }
}

/// A stale completion declaration on an automatic node must not change its file diff.
#[test]
fn automatic_node_ignores_stale_completion_for_its_file_changes() {
    let scaffold = ora_test_support::GitTestScaffold::new("backend-auto-completion-baseline")
        .expect("create Git test scaffold");
    let root = scaffold.repo_path();
    std::fs::create_dir_all(root.join("reports")).expect("reports directory");
    std::fs::write(root.join(".gitignore"), "reports/\n").expect("ignore reports");
    // Nine MiB of UTF-8 is above the targeted-capture inline threshold, so a wrongly targeted
    // baseline would store a digest while the ordinary current snapshot stores text.
    std::fs::write(root.join("large.txt"), "x".repeat(9 * 1024 * 1024)).expect("large text file");
    std::fs::write(root.join("reports/keep.md"), "unchanged\n").expect("ignored deliverable");

    let graph = WorkflowGraph::parse(
        r#"{"nodes":[
            {"id":"agent","data":{"kind":"agent","agentConfig":{
                "executor":{"agentCli":"open_code","modelId":"m"},
                "prompt":"do",
                "interactive":false,
                "schemaVersion":4,
                "completion":{"files":[
                    {"path":"large.txt","mustChange":true},
                    {"path":"reports/keep.md","mustChange":true}
                ]}
            }}}
        ],"edges":[]}"#,
    )
    .expect("parse automatic graph with a stale completion declaration");
    let config = graph
        .node("agent")
        .expect("agent node")
        .agent_config
        .as_ref()
        .expect("agent config");
    assert!(!config.interactive);
    assert!(config.completion.is_some());

    let baseline = capture_agent_baseline(root, config).expect("capture automatic baseline");
    assert_eq!(
        baseline.scope,
        BaselineScope::Full,
        "an automatic node must capture the ordinary Git-visible baseline"
    );
    let current = capture_worktree_snapshot(root);
    let file_changes = compute_file_changes(Some(&baseline), current.as_ref());
    assert_eq!(
        file_changes,
        Vec::new(),
        "unchanged declared files must not appear in an automatic node diff"
    );

    let recorder = Arc::new(RecordingCallback::default());
    let callback: Arc<dyn WorkflowRunCallback> = recorder.clone();
    report_outcome(
        &callback,
        &WorkflowRunId::new("run-1"),
        &WorkflowNodeRunId::new("node-1"),
        AgentNodeOutcome::Completed {
            output: Some("done".to_string()),
            structured_output: None,
            stop_reason: StopReason::EndTurn,
            file_changes,
        },
    );
    assert_eq!(
        *recorder
            .completions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![Vec::<FileChange>::new()],
        "the automatic completion callback receives no fabricated file changes"
    );
}
