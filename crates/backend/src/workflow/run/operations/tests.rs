use super::WorkflowRuns;
use crate::git_cleanup::KeyedResourceLocks;
use crate::workflow::run::test_fixture::{
    AGENT_GRAPH, RecordingExecutor, bind_and_park, bootstrap, init_git_workspace, run_test,
    started_run, started_run_with, write_open_turn, write_settled_turn,
};
use crate::workflow::run::worktree::{
    capture_completion_baseline, capture_worktree_snapshot, persist_worktree_baseline,
};
use crate::{Backend, test_backend::backend_paths};
use ora_application::{
    NodeFailure, NodeFailureKind, ResumeWorkflowRunResult, SessionRepository, WorkflowGraph,
    WorkflowRunEngineRepository, WorkflowRunRepository,
};
use ora_contracts::*;
use ora_db::{
    SqliteSessionRepository, SqliteWorkflowRunEngineRepository, SqliteWorkflowRunRepository,
};
use ora_domain::{SessionId, WorkflowNodeRunId};
use ora_history::history_path;
use pretty_assertions::assert_eq;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

struct Fixture {
    temporary: TempDir,
    backend: Backend,
    runs: Arc<WorkflowRuns>,
    node_run_id: String,
    /// The Git workspace the run executes in, so completion file checks read real files.
    workspace: PathBuf,
}

impl Fixture {
    /// Seeds an already-awaiting node without launching an external agent; operations thereafter
    /// cross the public interface backed by the production engine, runtime, and SQLite adapters.
    fn awaiting() -> Self {
        let temporary = TempDir::new().expect("run fixture");
        let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
            .expect("backend composition");
        let runs = backend.workflow_runs();
        let graph = AGENT_GRAPH.replace(
            "\"prompt\":\"do\"",
            "\"prompt\":\"do\",\"interactive\":true",
        );
        let (_, nodes) = started_run(&temporary, &runs.pool, &graph);
        let node = nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        let (_, node_run_id) = bind_and_park(&runs.pool, node);
        Self {
            workspace: temporary.path().join("fixture-project"),
            temporary,
            backend,
            runs,
            node_run_id: node_run_id.to_string(),
        }
    }

    /// Seeds an awaiting interactive node on a caller-supplied graph in a real Git workspace.
    ///
    /// `seed` runs after the Git repository exists and before the node starts, so the baseline the
    /// caller persists describes the same tree the completion check later inspects.
    fn awaiting_on_git_graph(graph: &str, seed: impl FnOnce(&std::path::Path)) -> Self {
        let temporary = TempDir::new().expect("run fixture");
        let workspace = init_git_workspace(&temporary);
        seed(&workspace);
        let backend = Backend::open(backend_paths(temporary.path(), temporary.path()))
            .expect("backend composition");
        let runs = backend.workflow_runs();
        let (_, nodes) = started_run(&temporary, &runs.pool, graph);
        let node = nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        let (_, node_run_id) = bind_and_park(&runs.pool, node);
        let parsed = WorkflowGraph::parse(graph).expect("parse fixture graph");
        let completion = parsed
            .execution_node("agent")
            .and_then(|node| node.agent_config.as_ref())
            .and_then(|config| config.completion.as_ref());
        let baseline =
            capture_completion_baseline(&workspace, completion).expect("capture fixture baseline");
        persist_worktree_baseline(&runs.baselines_root, &node_run_id, &baseline)
            .expect("persist fixture baseline");
        Self {
            workspace,
            temporary,
            backend,
            runs,
            node_run_id: node_run_id.to_string(),
        }
    }

    /// Loads the observable run and node projection through the same interface as Desktop.
    fn detail(&self) -> GetWorkflowRunResponse {
        self.runs
            .get(GetWorkflowRunRequest {
                run_id: "run-1".to_string(),
            })
            .expect("run detail")
    }

    /// Writes one settled turn so the completion gate sees a finished turn for the bound session.
    fn settle_turn(&self) {
        write_settled_turn(&self.runs.sessions_root, "session-1", "go", "Final answer");
    }

    /// Completes the node through the production use case, optionally forcing past blockers.
    async fn complete(
        &self,
        force: bool,
    ) -> Result<CompleteWorkflowNodeResponse, crate::BackendError> {
        self.runs
            .complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
                force,
                confirm_initial_turn: None,
                requester: Some(NodeCompletionRequester::Human),
            })
            .await
    }
}

/// Runs one Git command in a fixture workspace and returns trimmed stdout.
fn run_git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args([
            "-c",
            "user.name=ora-test",
            "-c",
            "user.email=ora-test@example.com",
        ])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// An interactive agent that declares two deliverables: one existence check and one change check.
const COMPLETION_AGENT_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"schemaVersion":4,"completion":{"files":[{"path":"reports/new.md"},{"path":"reports/keep.md","mustChange":true}]}}}}
],"edges":[{"source":"start","target":"agent"}]}"#;

/// The same graph with only a `mustChange` declaration, used for baseline degradation.
const COMPLETION_MUST_CHANGE_GRAPH: &str = r#"{"nodes":[
    {"id":"start","data":{"kind":"start"}},
    {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"schemaVersion":4,"completion":{"files":[{"path":"reports/keep.md","mustChange":true}]}}}}
],"edges":[{"source":"start","target":"agent"}]}"#;

/// Parses one node run's payload JSON, which is where the read projection finds `completion`.
fn node_payload(node: &WorkflowNodeRun) -> serde_json::Value {
    serde_json::from_str(node.payload.as_deref().expect("node payload")).expect("node payload JSON")
}

/// Maps one completion rejection onto its blockers, failing on any other public error.
fn incomplete_blockers(error: crate::BackendError) -> Vec<WorkflowNodeCompletionBlocker> {
    match error.public_error() {
        PublicError::WorkflowNodeIncomplete(params) => params.blockers.clone(),
        other => panic!("expected workflow_node_incomplete, got {other:?}"),
    }
}

/// A missing path blocks first; after it is created, an unchanged path blocks; then it completes.
#[test]
fn completion_files_block_missing_and_unchanged_then_allow_a_retry() {
    run_test(async {
        let fixture = Fixture::awaiting_on_git_graph(COMPLETION_AGENT_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join("reports/keep.md"), "original\n").expect("keep fixture");
        });
        fixture.settle_turn();

        let blockers = incomplete_blockers(
            fixture
                .complete(false)
                .await
                .expect_err("missing and unchanged deliverables block completion"),
        );
        assert_eq!(
            blockers,
            vec![
                WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::FileMissing,
                    message: None,
                    params: Some(WorkflowNodeCompletionBlockerParams {
                        path: Some("reports/new.md".to_string()),
                    }),
                },
                WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::FileUnchanged,
                    message: None,
                    params: Some(WorkflowNodeCompletionBlockerParams {
                        path: Some("reports/keep.md".to_string()),
                    }),
                },
            ]
        );
        // The rejected completion left the node awaiting input and wrote no file blockers into the
        // snapshot the projection reads.
        let detail = fixture.detail();
        assert_eq!(detail.run.status, WorkflowRunStatus::AwaitingInput);
        let node = detail
            .nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        let payload = node_payload(node);
        assert_eq!(
            payload["completion"]["blockers"],
            serde_json::json!([]),
            "file blockers are computed at completion and never written into the park snapshot"
        );
        assert!(
            !payload["completion"]["warnings"]
                .as_array()
                .expect("warnings array")
                .iter()
                .any(|warning| warning == "file_check_unavailable"),
            "a present baseline must not warn"
        );

        fs::write(fixture.workspace.join("reports/new.md"), "fresh\n").expect("add deliverable");
        let blockers = incomplete_blockers(
            fixture
                .complete(false)
                .await
                .expect_err("the unchanged deliverable still blocks"),
        );
        assert_eq!(
            blockers,
            vec![WorkflowNodeCompletionBlocker {
                kind: WorkflowNodeCompletionBlockerKind::FileUnchanged,
                message: None,
                params: Some(WorkflowNodeCompletionBlockerParams {
                    path: Some("reports/keep.md".to_string()),
                }),
            }]
        );

        fs::write(
            fixture.workspace.join("reports/keep.md"),
            "original\nupdated\n",
        )
        .expect("change deliverable");
        let response = fixture
            .complete(false)
            .await
            .expect("both deliverables now satisfy the declaration");
        assert_eq!(response.run.status, WorkflowRunStatus::Succeeded);
        // A fresh turn is not needed: the retry after user edits completes through the same entry.
        let node = fixture
            .detail()
            .nodes
            .into_iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        assert_eq!(node.status, WorkflowNodeStatus::Succeeded);
    });
}

/// File blockers join the forced audit while staying out of the persisted completion snapshot.
#[test]
fn forcing_a_completion_records_file_blockers_in_its_audit() {
    run_test(async {
        let fixture = Fixture::awaiting_on_git_graph(COMPLETION_AGENT_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join("reports/keep.md"), "original\n").expect("keep fixture");
        });
        fixture.settle_turn();

        let response = fixture
            .complete(true)
            .await
            .expect("force overrides file blockers");
        assert_eq!(response.run.status, WorkflowRunStatus::Succeeded);

        let node = SqliteWorkflowRunRepository::new(fixture.runs.pool.clone())
            .list_node_runs(&ora_domain::WorkflowRunId::new("run-1"))
            .expect("node runs")
            .into_iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        let payload: serde_json::Value =
            serde_json::from_str(node.payload.as_deref().expect("payload")).expect("payload JSON");
        assert_eq!(
            payload["completion_forced"]["blockers"],
            serde_json::json!([
                { "kind": "file_missing", "params": { "path": "reports/new.md" } },
                { "kind": "file_unchanged", "params": { "path": "reports/keep.md" } }
            ])
        );
        assert_eq!(
            payload["completion"]["blockers"],
            serde_json::json!([]),
            "file blockers are computed at completion and never written into the park snapshot"
        );
    });
}

/// A missing baseline warns at completion, skips `mustChange`, and the warning survives success.
#[test]
fn a_missing_baseline_degrades_must_change_to_a_visible_warning() {
    run_test(async {
        let fixture = Fixture::awaiting_on_git_graph(COMPLETION_MUST_CHANGE_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join("reports/keep.md"), "original\n").expect("keep fixture");
        });
        // Simulate a pre-upgrade node whose baseline was never persisted.
        fs::remove_file(
            fixture
                .runs
                .baselines_root
                .join(format!("{}.json", fixture.node_run_id)),
        )
        .expect("remove baseline");
        fixture.settle_turn();

        let response = fixture
            .complete(false)
            .await
            .expect("mustChange cannot be judged without a baseline, so it does not block");
        assert_eq!(response.run.status, WorkflowRunStatus::Succeeded);

        let node = fixture
            .detail()
            .nodes
            .into_iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        let payload = node_payload(&node);
        assert!(
            payload["completion"]["warnings"]
                .as_array()
                .expect("warnings array")
                .iter()
                .any(|warning| warning == "file_check_unavailable"),
            "the warning must be visible in the read projection, not only an error response"
        );
    });
}

/// Existence still blocks without a baseline; only the undecidable change requirement degrades.
#[test]
fn a_missing_baseline_still_checks_declared_file_existence() {
    run_test(async {
        let fixture = Fixture::awaiting_on_git_graph(COMPLETION_AGENT_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join("reports/keep.md"), "original\n").expect("keep fixture");
        });
        fs::remove_file(
            fixture
                .runs
                .baselines_root
                .join(format!("{}.json", fixture.node_run_id)),
        )
        .expect("remove baseline");
        fixture.settle_turn();

        let blockers = incomplete_blockers(
            fixture
                .complete(false)
                .await
                .expect_err("the missing path still blocks without a baseline"),
        );
        assert_eq!(
            blockers,
            vec![WorkflowNodeCompletionBlocker {
                kind: WorkflowNodeCompletionBlockerKind::FileMissing,
                message: None,
                params: Some(WorkflowNodeCompletionBlockerParams {
                    path: Some("reports/new.md".to_string()),
                }),
            }]
        );

        fs::write(fixture.workspace.join("reports/new.md"), "fresh\n").expect("add deliverable");
        fixture
            .complete(false)
            .await
            .expect("the existing path and undecidable change requirement allow completion");
    });
}

/// A baseline that becomes corrupt after the park still gets the warning added on completion.
#[test]
fn a_baseline_corrupted_after_park_warns_before_completing() {
    run_test(async {
        let fixture = Fixture::awaiting_on_git_graph(COMPLETION_MUST_CHANGE_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join("reports/keep.md"), "original\n").expect("keep fixture");
        });
        fixture.settle_turn();
        let ready = serde_json::json!({
            "completion": {
                "userSeq": 0,
                "turnKey": { "userSeq": 0, "endedSeq": 2 },
                "stopReason": "end_turn",
                "turnOrigin": "human",
                "blockers": [],
                "warnings": [],
                "outputDigest": format!(
                    "sha256:{}",
                    ora_utils::hash::sha256_hex(b"Final answer")
                ),
                "provisional": false,
            }
        });
        assert!(
            SqliteWorkflowRunEngineRepository::new(fixture.runs.pool.clone())
                .repair_node_payload(&WorkflowNodeRunId::new(&fixture.node_run_id), &ready, 60)
                .expect("seed the ready completion snapshot")
        );
        fs::write(
            fixture
                .runs
                .baselines_root
                .join(format!("{}.json", fixture.node_run_id)),
            "{not json",
        )
        .expect("corrupt baseline after park");
        fixture
            .complete(false)
            .await
            .expect("undecidable mustChange does not block completion");
        let node = fixture
            .detail()
            .nodes
            .into_iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        let payload = node_payload(&node);
        assert!(
            payload["completion"]["warnings"]
                .as_array()
                .expect("warnings array")
                .iter()
                .any(|warning| warning == "file_check_unavailable"),
            "completion must repair the parked snapshot with the warning it observed"
        );
    });
}

/// `*`, `**`, and `?` match their documented segments against a real workspace snapshot.
#[test]
fn completion_globs_match_files_in_the_run_workspace() {
    run_test(async {
        const GLOB_GRAPH: &str = r#"{"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"schemaVersion":4,"completion":{"files":[{"path":"docs/*.md"},{"path":"docs/**/b.md"},{"path":"docs/?.md"}]}}}}
        ],"edges":[{"source":"start","target":"agent"}]}"#;
        let fixture = Fixture::awaiting_on_git_graph(GLOB_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("docs/nested")).expect("docs directory");
            fs::write(workspace.join("docs/a.md"), "a\n").expect("a fixture");
            fs::write(workspace.join("docs/nested/b.md"), "b\n").expect("b fixture");
        });
        fixture.settle_turn();
        fixture
            .complete(false)
            .await
            .expect("every supported wildcard finds its matching file");
    });
}

/// Glob matching is case-sensitive, so a differently-cased directory does not satisfy a path.
#[test]
fn completion_globs_are_case_sensitive() {
    run_test(async {
        const CASE_GRAPH: &str = r#"{"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"schemaVersion":4,"completion":{"files":[{"path":"DOCS/report.md"}]}}}}
        ],"edges":[{"source":"start","target":"agent"}]}"#;
        let fixture = Fixture::awaiting_on_git_graph(CASE_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("docs")).expect("docs directory");
            fs::write(workspace.join("docs/report.md"), "report\n").expect("report fixture");
        });
        fixture.settle_turn();
        let blockers = incomplete_blockers(
            fixture
                .complete(false)
                .await
                .expect_err("a case-different path does not match"),
        );
        assert_eq!(
            blockers,
            vec![WorkflowNodeCompletionBlocker {
                kind: WorkflowNodeCompletionBlockerKind::FileMissing,
                message: None,
                params: Some(WorkflowNodeCompletionBlockerParams {
                    path: Some("DOCS/report.md".to_string()),
                }),
            }]
        );
    });
}

/// A symbolic link whose target escapes the workspace is not a declared deliverable.
#[cfg(unix)]
#[test]
fn completion_files_do_not_follow_a_symlink_out_of_the_workspace() {
    run_test(async {
        const SYMLINK_GRAPH: &str = r#"{"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"schemaVersion":4,"completion":{"files":[{"path":"reports/escape.md"}]}}}}
        ],"edges":[{"source":"start","target":"agent"}]}"#;
        let outside = TempDir::new().expect("outside fixture");
        fs::write(outside.path().join("secret.md"), "outside\n").expect("outside file");
        let outside_file = outside.path().join("secret.md");
        let fixture = Fixture::awaiting_on_git_graph(SYMLINK_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            std::os::unix::fs::symlink(&outside_file, workspace.join("reports/escape.md"))
                .expect("escape symlink");
        });
        fixture.settle_turn();
        let blockers = incomplete_blockers(
            fixture
                .complete(false)
                .await
                .expect_err("an escaping symlink does not count as existence"),
        );
        assert_eq!(
            blockers,
            vec![WorkflowNodeCompletionBlocker {
                kind: WorkflowNodeCompletionBlockerKind::FileMissing,
                message: None,
                params: Some(WorkflowNodeCompletionBlockerParams {
                    path: Some("reports/escape.md".to_string()),
                }),
            }]
        );
    });
}

/// A binary deliverable is unchanged until its bytes differ, even though neither side is text.
#[test]
fn completion_must_change_detects_binary_content_changes() {
    run_test(async {
        const BINARY_GRAPH: &str = r#"{"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"schemaVersion":4,"completion":{"files":[{"path":"assets/report.bin","mustChange":true}]}}}}
        ],"edges":[{"source":"start","target":"agent"}]}"#;
        let fixture = Fixture::awaiting_on_git_graph(BINARY_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("assets")).expect("assets directory");
            fs::write(workspace.join("assets/report.bin"), [0, 1, 2, 3]).expect("binary fixture");
        });
        fixture.settle_turn();

        let blockers = incomplete_blockers(
            fixture
                .complete(false)
                .await
                .expect_err("identical binary content is unchanged"),
        );
        assert_eq!(
            blockers,
            vec![WorkflowNodeCompletionBlocker {
                kind: WorkflowNodeCompletionBlockerKind::FileUnchanged,
                message: None,
                params: Some(WorkflowNodeCompletionBlockerParams {
                    path: Some("assets/report.bin".to_string()),
                }),
            }]
        );

        // Same length, different bytes: a text-only snapshot would compare None == None here.
        fs::write(fixture.workspace.join("assets/report.bin"), [0, 1, 2, 4])
            .expect("change binary content");
        fixture
            .complete(false)
            .await
            .expect("binary content change satisfies mustChange");
    });
}

/// A gitlink or directory is not a file deliverable even though git lists the former.
#[test]
fn completion_files_do_not_accept_a_gitlink_or_directory() {
    run_test(async {
        const DIRECTORY_GRAPH: &str = r#"{"nodes":[
            {"id":"start","data":{"kind":"start"}},
            {"id":"agent","data":{"kind":"agent","agentConfig":{"retry":{"enabled":false,"maxRetries":0,"initialDelaySeconds":0},"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"do","interactive":true,"schemaVersion":4,"completion":{"files":[{"path":"vendor/module"},{"path":"reports"}]}}}}
        ],"edges":[{"source":"start","target":"agent"}]}"#;
        let fixture = Fixture::awaiting_on_git_graph(DIRECTORY_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join("reports/keep.md"), "keep\n").expect("reports fixture");
            fs::create_dir_all(workspace.join("vendor/module")).expect("gitlink directory");
            let head = run_git(workspace, &["rev-parse", "HEAD"]);
            run_git(
                workspace,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("160000,{head},vendor/module"),
                ],
            );
        });
        fixture.settle_turn();

        let blockers = incomplete_blockers(
            fixture
                .complete(false)
                .await
                .expect_err("a gitlink and a directory are not deliverable files"),
        );
        assert_eq!(
            blockers,
            vec![
                WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::FileMissing,
                    message: None,
                    params: Some(WorkflowNodeCompletionBlockerParams {
                        path: Some("vendor/module".to_string()),
                    }),
                },
                WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::FileMissing,
                    message: None,
                    params: Some(WorkflowNodeCompletionBlockerParams {
                        path: Some("reports".to_string()),
                    }),
                },
            ]
        );
    });
}

/// A corrupt baseline is unavailable: it warns and skips only the undecidable mustChange check.
#[test]
fn a_corrupt_baseline_warns_and_degrades_must_change() {
    run_test(async {
        let fixture = Fixture::awaiting_on_git_graph(COMPLETION_MUST_CHANGE_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join("reports/keep.md"), "original\n").expect("keep fixture");
        });
        fs::write(
            fixture
                .runs
                .baselines_root
                .join(format!("{}.json", fixture.node_run_id)),
            "{not json",
        )
        .expect("corrupt baseline");
        fixture.settle_turn();

        fixture
            .complete(false)
            .await
            .expect("a corrupt baseline cannot decide mustChange, so it does not block");
        let node = fixture
            .detail()
            .nodes
            .into_iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        let payload = node_payload(&node);
        assert!(
            payload["completion"]["warnings"]
                .as_array()
                .expect("warnings array")
                .iter()
                .any(|warning| warning == "file_check_unavailable"),
            "a corrupt baseline must be visible as unavailable, not silently treated as present"
        );
    });
}

/// Concurrent public completions commit once, retain the final assistant output, and clean up
/// the owned baseline; a later prompt cannot reopen the terminal workflow node.
#[test]
fn concurrent_completions_commit_once_and_leave_the_session_read_only() {
    run_test(async {
        let fixture = Fixture::awaiting();
        fixture.settle_turn();
        fs::create_dir_all(&fixture.runs.baselines_root).expect("baseline directory");
        let baseline = fixture
            .runs
            .baselines_root
            .join(format!("{}.json", fixture.node_run_id));
        fs::write(&baseline, "{}").expect("worktree baseline");
        let request = CompleteWorkflowNodeRequest {
            run_id: "run-1".to_string(),
            node_id: "agent".to_string(),
            force: false,
            confirm_initial_turn: None,
            requester: Some(NodeCompletionRequester::Human),
        };
        let (first, second) = tokio::join!(
            fixture.runs.complete_node(request.clone()),
            fixture.runs.complete_node(request)
        );
        let (success, error) = match (first, second) {
            (Ok(success), Err(error)) | (Err(error), Ok(success)) => (success, error),
            outcomes => panic!("exactly one completion must succeed: {outcomes:?}"),
        };
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {})
        );
        let detail = fixture.detail();
        assert_eq!(
            success,
            CompleteWorkflowNodeResponse {
                run: detail.run.clone()
            }
        );
        assert_eq!(detail.run.status, WorkflowRunStatus::Succeeded);
        let agent = detail
            .nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .expect("node");
        assert_eq!(
            (agent.status, agent.output.as_deref()),
            (WorkflowNodeStatus::Succeeded, Some("Final answer"))
        );
        assert!(
            !baseline.exists(),
            "a committed node no longer owns its baseline"
        );
        let error = fixture
            .backend
            .sessions()
            .prompt(PromptSessionRequest {
                session_id: "session-1".to_string(),
                prompt: vec![],
                record_prompt: None,
                model: None,
            })
            .await
            .err()
            .expect("terminal node rejects prompt before contacting a provider");
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {})
        );
    });
}

/// A real history read failure leaves the run unchanged and releases its transient claim so a
/// repaired history can be completed through the same interface without restarting the app.
#[test]
fn prepare_failure_releases_the_claim_for_retry() {
    run_test(async {
        let fixture = Fixture::awaiting();
        let before = fixture.detail();
        let history = history_path(&fixture.runs.sessions_root, "session-1").expect("history path");
        fs::create_dir_all(&history).expect("inject unreadable history path");
        let request = CompleteWorkflowNodeRequest {
            run_id: "run-1".to_string(),
            node_id: "agent".to_string(),
            force: false,
            confirm_initial_turn: None,
            requester: Some(NodeCompletionRequester::Human),
        };
        let error = fixture
            .runs
            .complete_node(request.clone())
            .await
            .expect_err("history cannot be read");
        assert_eq!(error.classification(), crate::ErrorClassification::Internal);
        assert_eq!(fixture.detail(), before);
        fs::remove_dir(&history).expect("repair fixture history path");
        fixture.settle_turn();
        let completed = fixture
            .runs
            .complete_node(request)
            .await
            .expect("claim released for retry");
        assert_eq!(completed.run.status, WorkflowRunStatus::Succeeded);
    });
}

/// A turn that never settled is rejected with the blocker the user has to resolve.
#[test]
fn an_unsettled_completion_is_rejected_with_its_blockers() {
    run_test(async {
        let fixture = Fixture::awaiting();
        write_open_turn(&fixture.runs.sessions_root, "session-1", "go", "draft only");
        let error = fixture
            .runs
            .complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
                force: false,
                confirm_initial_turn: None,
                requester: Some(NodeCompletionRequester::Human),
            })
            .await
            .expect_err("an unsettled turn cannot complete without force");
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeIncomplete(WorkflowNodeIncompleteParams {
                node_id: "agent".to_string(),
                blockers: vec![WorkflowNodeCompletionBlocker {
                    kind: WorkflowNodeCompletionBlockerKind::TurnUnknown,
                    message: None,
                    params: None,
                }],
            })
        );
        // The rejected completion left the node awaiting input and recorded no audit.
        assert_eq!(
            fixture.detail().run.status,
            WorkflowRunStatus::AwaitingInput
        );
    });
}

/// A forced completion commits its audit in the same payload as the completion.
#[test]
fn a_forced_completion_records_its_audit_in_the_completed_payload() {
    run_test(async {
        let fixture = Fixture::awaiting();
        write_open_turn(&fixture.runs.sessions_root, "session-1", "go", "draft only");
        let response = fixture
            .runs
            .complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
                force: true,
                confirm_initial_turn: None,
                requester: Some(NodeCompletionRequester::Human),
            })
            .await
            .expect("a forced completion commits");
        assert_eq!(response.run.status, WorkflowRunStatus::Succeeded);

        let node = SqliteWorkflowRunRepository::new(fixture.runs.pool.clone())
            .list_node_runs(&ora_domain::WorkflowRunId::new("run-1"))
            .expect("node runs")
            .into_iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        assert_eq!(node.output.as_deref(), Some("draft only"));
        let payload: serde_json::Value =
            serde_json::from_str(node.payload.as_deref().expect("payload")).expect("payload JSON");
        assert_eq!(
            payload["completion_forced"],
            serde_json::json!({
                "blockers": [{ "kind": "turn_unknown" }],
                "stop_cause": null,
                "without_turn": true,
                "output_source": "last_assistant_message",
            }),
            "the forced audit is part of the completion payload, not a later write"
        );
    });
}

/// Forcing a ready turn is an ordinary completion: it writes no empty forced audit.
///
/// `force` only overrides a blocker that exists. A snapshot whose turn settled normally has nothing
/// to override, so the completion must take the plain path and leave `payload.completion_forced`
/// absent instead of recording an override that never happened (ADR D5).
#[test]
fn ready_force_does_not_write_an_empty_audit() {
    run_test(async {
        let fixture = Fixture::awaiting();
        fixture.settle_turn();
        // Seed the ready snapshot exactly as a plain park of the settled turn writes it, so the
        // forced request is applied to a snapshot that already has nothing to override.
        let ready = serde_json::json!({
            "completion": {
                "userSeq": 0,
                "turnKey": { "userSeq": 0, "endedSeq": 2 },
                "stopReason": "end_turn",
                "turnOrigin": "human",
                "blockers": [],
                "warnings": [],
                "outputDigest": format!(
                    "sha256:{}",
                    ora_utils::hash::sha256_hex(b"Final answer")
                ),
                "provisional": false,
            }
        });
        assert!(
            SqliteWorkflowRunEngineRepository::new(fixture.runs.pool.clone())
                .repair_node_payload(&WorkflowNodeRunId::new(&fixture.node_run_id), &ready, 60)
                .expect("seed the ready completion snapshot")
        );

        let response = fixture
            .runs
            .complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
                force: true,
                confirm_initial_turn: None,
                requester: Some(NodeCompletionRequester::Human),
            })
            .await
            .expect("force over a ready turn completes as a plain completion");
        assert_eq!(response.run.status, WorkflowRunStatus::Succeeded);

        let node = SqliteWorkflowRunRepository::new(fixture.runs.pool.clone())
            .list_node_runs(&ora_domain::WorkflowRunId::new("run-1"))
            .expect("node runs")
            .into_iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        assert_eq!(node.output.as_deref(), Some("Final answer"));
        let payload: serde_json::Value =
            serde_json::from_str(node.payload.as_deref().expect("payload")).expect("payload JSON");
        assert_eq!(
            payload.get("completion_forced"),
            None,
            "a ready force completion records no forced audit"
        );
        assert_eq!(
            payload["completion"], ready["completion"],
            "the ready snapshot stays the record of the park it completed"
        );
    });
}

/// A request that both confirms the opening turn and forces has no consistent meaning.
#[test]
fn conflicting_completion_flags_are_rejected_before_claiming() {
    run_test(async {
        let fixture = Fixture::awaiting();
        let before = fixture.detail();
        let error = fixture
            .runs
            .complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
                force: true,
                confirm_initial_turn: Some(true),
                requester: Some(NodeCompletionRequester::Human),
            })
            .await
            .expect_err("force and confirmation contradict each other");
        assert_eq!(
            error.public_error(),
            &PublicError::InvalidRequest(EmptyErrorParams {})
        );
        assert_eq!(
            fixture.detail(),
            before,
            "the rejection happens before the node is claimed"
        );
    });
}

/// A prompt rejected by the runtime restores the workflow node before returning the error.
#[test]
fn failed_prompt_start_restores_the_awaiting_node_before_returning() {
    run_test(async {
        let fixture = Fixture::awaiting();
        let error = fixture
            .backend
            .sessions()
            .prompt(PromptSessionRequest {
                session_id: "session-1".to_string(),
                prompt: Vec::new(),
                record_prompt: None,
                model: None,
            })
            .await
            .err()
            .expect("empty prompt is rejected after workflow admission");
        assert_eq!(
            error.public_error(),
            &PublicError::PromptEmpty(EmptyErrorParams {})
        );
        assert_eq!(
            fixture.detail().run.status,
            WorkflowRunStatus::AwaitingInput
        );
        fixture.settle_turn();
        let completed = fixture
            .runs
            .complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
                force: false,
                confirm_initial_turn: None,
                requester: None,
            })
            .await
            .expect("the restored awaiting node can be completed immediately");
        assert_eq!(completed.run.status, WorkflowRunStatus::Succeeded);
    });
}

/// A session that disappeared during cancellation cannot undo the committed terminal transition.
#[test]
fn cancellation_commits_even_when_session_cleanup_fails() {
    run_test(async {
        let fixture = Fixture::awaiting();
        SqliteSessionRepository::new(fixture.runs.pool.clone())
            .soft_delete_session(&SessionId::new("session-1"), 60)
            .expect("session disappears");
        let response = fixture
            .runs
            .cancel(CancelWorkflowRunRequest {
                run_id: "run-1".to_string(),
            })
            .await
            .expect("cancel remains successful");
        let detail = fixture.detail();
        assert_eq!(
            response,
            CancelWorkflowRunResponse {
                run: detail.run.clone()
            }
        );
        assert_eq!(detail.run.status, WorkflowRunStatus::Cancelled);
        let error = fixture
            .runs
            .complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
                force: false,
                confirm_initial_turn: None,
                requester: Some(NodeCompletionRequester::Human),
            })
            .await
            .expect_err("cancelled node cannot complete");
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {})
        );
        assert_eq!(fixture.detail(), detail);
    });
}

/// Reopening the full composition preserves a genuine awaiting node and removes only orphaned
/// baseline files; this exercises the actual startup path rather than a recovery predicate.
#[test]
fn recovery_preserves_awaiting_nodes_and_prunes_orphaned_baselines() {
    run_test(async {
        let fixture = Fixture::awaiting();
        let expected = fixture.detail();
        fs::create_dir_all(&fixture.runs.baselines_root).expect("baseline directory");
        let baseline = fixture
            .runs
            .baselines_root
            .join(format!("{}.json", fixture.node_run_id));
        let orphan = fixture.runs.baselines_root.join("missing-node.json");
        let unrelated = fixture.runs.baselines_root.join("notes.txt");
        for path in [&baseline, &orphan, &unrelated] {
            fs::write(path, "{}").expect("seed side file");
        }
        let Fixture {
            temporary,
            backend,
            runs,
            ..
        } = fixture;
        drop(runs);
        drop(backend);
        let reopened = Backend::open(backend_paths(temporary.path(), temporary.path()))
            .expect("reopened backend");
        assert_eq!(
            reopened
                .workflow_runs()
                .get(GetWorkflowRunRequest {
                    run_id: "run-1".to_string()
                })
                .expect("recovered run"),
            expected
        );
        assert_eq!(
            (baseline.is_file(), orphan.exists(), unrelated.is_file()),
            (true, false, true)
        );
    });
}

/// Both public paths use the same run gate: cancellation either wins before completion's commit
/// or observes the already-succeeded run, never reporting a completion of a cancelled node.
#[test]
fn concurrent_cancel_and_complete_report_the_committed_outcome() {
    run_test(async {
        let fixture = Fixture::awaiting();
        fixture.settle_turn();
        let (cancelled, completed) = tokio::join!(
            fixture.runs.cancel(CancelWorkflowRunRequest {
                run_id: "run-1".to_string()
            }),
            fixture.runs.complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
                force: false,
                confirm_initial_turn: None,
                requester: None,
            }),
        );
        let detail = fixture.detail();
        assert_eq!(
            cancelled.expect("cancel is idempotent for terminal runs"),
            CancelWorkflowRunResponse {
                run: detail.run.clone()
            }
        );
        match completed {
            Ok(response) => {
                assert_eq!(
                    response,
                    CompleteWorkflowNodeResponse {
                        run: detail.run.clone()
                    }
                );
                assert_eq!(detail.run.status, WorkflowRunStatus::Succeeded);
            }
            Err(error) => {
                assert_eq!(
                    error.public_error(),
                    &PublicError::WorkflowNodeNotAwaitingInput(EmptyErrorParams {})
                );
                assert_eq!(detail.run.status, WorkflowRunStatus::Cancelled);
            }
        }
    });
}

/// Startup fails an interrupted turn but resumes a run whose last node committed before a crash
/// could schedule its successor; both cases exercise the recovery wired by Backend::open.
#[test]
fn recovery_fails_interrupted_turns_and_resumes_stalled_runs() {
    run_test(async {
        for (persisted_node_status, recovered_run_status) in [
            (
                ora_domain::WorkflowNodeStatus::Running,
                WorkflowRunStatus::Failed,
            ),
            (
                ora_domain::WorkflowNodeStatus::Succeeded,
                WorkflowRunStatus::Succeeded,
            ),
        ] {
            let fixture = Fixture::awaiting();
            SqliteWorkflowRunEngineRepository::new(fixture.runs.pool.clone())
                .transition_node_run_status(
                    &WorkflowNodeRunId::new(&fixture.node_run_id),
                    ora_domain::WorkflowNodeStatus::Pending,
                    persisted_node_status,
                    70,
                )
                .expect("persist the crash point");
            let Fixture {
                temporary,
                backend,
                runs,
                ..
            } = fixture;
            drop(runs);
            drop(backend);
            let reopened = Backend::open(backend_paths(temporary.path(), temporary.path()))
                .expect("reopened backend");
            let detail = reopened
                .workflow_runs()
                .get(GetWorkflowRunRequest {
                    run_id: "run-1".to_string(),
                })
                .expect("recovered run");
            assert_eq!(detail.run.status, recovered_run_status);
        }
    });
}

/// B10: exclusive run lock admits exactly one concurrent resume.
///
/// Driven through the same lock-then-resume protocol as `WorkflowRuns::resume_from_failure`,
/// but with `RecordingExecutor` so the rerun node stays `Running`. The Backend composition has
/// no executor seam; its real dispatcher fails the agent within milliseconds and would make a
/// second serialized resume legitimate product behaviour.
#[test]
fn concurrent_resume_from_failure_reruns_the_failed_node_once() {
    ora_logging::with_trace_logging(|| {
        let (temporary, pool) = bootstrap();
        let (run_id, nodes, engine) =
            started_run_with(&temporary, &pool, AGENT_GRAPH, RecordingExecutor::default());
        let agent = nodes
            .iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node")
            .clone();
        engine
            .fail_node(
                &run_id,
                &agent.id,
                NodeFailure::new(NodeFailureKind::Session, "agent failed"),
            )
            .unwrap();
        let locks = KeyedResourceLocks::new();
        let start = std::sync::Barrier::new(2);
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                start.wait();
                let _gate = locks.acquire_exclusive(run_id.as_ref());
                engine.resume_from_failure(&run_id)
            });
            let second = scope.spawn(|| {
                start.wait();
                let _gate = locks.acquire_exclusive(run_id.as_ref());
                engine.resume_from_failure(&run_id)
            });
            (
                first.join().expect("join first"),
                second.join().expect("join second"),
            )
        });
        let outcomes = [first, second].map(|result| match result.expect("resume") {
            ResumeWorkflowRunResult::Resumed => Ok(()),
            ResumeWorkflowRunResult::NotResumable => {
                Err(PublicError::WorkflowRunNotResumable(EmptyErrorParams {}))
            }
            ResumeWorkflowRunResult::NotFound => {
                Err(PublicError::WorkflowRunNotFound(EmptyErrorParams {}))
            }
        });
        let resumed = outcomes.iter().filter(|result| result.is_ok()).count();
        let refused = outcomes
            .iter()
            .filter(|result| {
                result.as_ref().is_err_and(|error| {
                    matches!(
                        error,
                        PublicError::WorkflowRunNotResumable(_) | PublicError::WorkflowRunActive(_)
                    )
                })
            })
            .count();
        assert_eq!((resumed, refused), (1, 1), "{outcomes:?}");
        let live = SqliteWorkflowRunRepository::new(pool)
            .list_node_runs(&run_id)
            .unwrap();
        let agent_live: Vec<_> = live.iter().filter(|node| node.node_id == "agent").collect();
        assert_eq!(agent_live.len(), 1);
        assert_ne!(agent_live[0].id.as_ref(), agent.id.as_ref());
        assert_eq!(
            agent_live[0].status,
            ora_domain::WorkflowNodeStatus::Running
        );
    });
}

/// The full startup composition drains an interrupted break and resumes only the outer graph.
#[test]
fn loop_exit_recovery_finishes_bound_session_cleanup() {
    run_test(async {
        let temporary = TempDir::new().unwrap();
        let backend = Backend::open(backend_paths(temporary.path(), temporary.path())).unwrap();
        let runs = backend.workflow_runs();
        let graph = crate::workflow::run::loop_exit_tests::graph(/*parallel*/ true);
        let (id, nodes, engine) = started_run_with(
            &temporary,
            &runs.pool,
            &graph,
            crate::workflow::run::loop_exit_tests::PendingCleanup,
        );
        let writer = nodes.iter().find(|node| node.node_id == "writer").unwrap();
        let slow = nodes.iter().find(|node| node.node_id == "slow").unwrap();
        let _ = bind_and_park(&runs.pool, slow);
        engine
            .complete_node(
                &id,
                &writer.id,
                Some("done".into()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                vec![],
            )
            .unwrap();
        drop(engine);
        drop(runs);
        drop(backend);
        let reopened = Backend::open(backend_paths(temporary.path(), temporary.path())).unwrap();
        let runs = reopened.workflow_runs();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let detail = runs
                    .get(GetWorkflowRunRequest {
                        run_id: id.to_string(),
                    })
                    .unwrap();
                if detail.run.status == ora_contracts::WorkflowRunStatus::Succeeded {
                    assert_eq!(detail.run.output, Some(r#"{"result":"done"}"#.into()));
                    break;
                }
                assert_ne!(detail.run.status, ora_contracts::WorkflowRunStatus::Failed);
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("exit acknowledgement resumes the outer graph");
    });
}

/// A snapshot budget failure cannot turn an existing deliverable into a missing file.
#[test]
fn an_unrelated_oversized_file_does_not_hide_an_existing_deliverable() {
    run_test(async {
        let graph =
            COMPLETION_MUST_CHANGE_GRAPH.replace("\"mustChange\":true", "\"mustChange\":false");
        let fixture = Fixture::awaiting_on_git_graph(&graph, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join("reports/keep.md"), "deliverable\n").expect("deliverable");
        });
        fs::File::create(fixture.workspace.join("oversized-unrelated.bin"))
            .expect("unrelated file")
            .set_len(65 * 1024 * 1024)
            .expect("exceed the documented snapshot budget without allocating contents");
        assert!(capture_worktree_snapshot(&fixture.workspace).is_none());
        fixture.settle_turn();
        let completed = fixture.complete(false).await.expect(
            "existence remains checkable when unrelated contents exceed the snapshot budget",
        );
        assert_eq!(completed.run.status, WorkflowRunStatus::Succeeded);
    });
}

/// Declaring a real artifact is an existence check even when Git ignores the output directory.
#[test]
fn an_existing_gitignored_deliverable_satisfies_its_existence_declaration() {
    run_test(async {
        let graph =
            COMPLETION_MUST_CHANGE_GRAPH.replace("\"mustChange\":true", "\"mustChange\":false");
        let fixture = Fixture::awaiting_on_git_graph(&graph, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join(".gitignore"), "reports/\n").expect("ignored outputs");
            fs::write(workspace.join("reports/keep.md"), "deliverable\n").expect("deliverable");
        });
        fixture.settle_turn();
        let completed = fixture
            .complete(false)
            .await
            .expect("a declared file that exists must not be hidden by Git ignore rules");
        assert_eq!(completed.run.status, WorkflowRunStatus::Succeeded);
    });
}

/// A legacy null binary entry contains no digest and cannot prove a change after upgrade.
#[test]
fn a_legacy_binary_baseline_degrades_with_a_visible_warning() {
    run_test(async {
        let graph = COMPLETION_MUST_CHANGE_GRAPH.replace("reports/keep.md", "artifact.bin");
        let fixture = Fixture::awaiting_on_git_graph(&graph, |workspace| {
            fs::write(workspace.join("artifact.bin"), [0xff, 0x00, 0x80])
                .expect("pre-existing binary deliverable");
        });
        fs::write(
            fixture
                .runs
                .baselines_root
                .join(format!("{}.json", fixture.node_run_id)),
            serde_json::to_vec(&serde_json::json!({ "artifact.bin": null }))
                .expect("legacy text-only snapshot"),
        )
        .expect("restore a legacy baseline that never recorded binary content");
        fixture.settle_turn();
        let completed = fixture
            .complete(false)
            .await
            .expect("an unknowable legacy change check degrades instead of inventing a comparison");
        assert_eq!(completed.run.status, WorkflowRunStatus::Succeeded);
        let node = fixture
            .detail()
            .nodes
            .into_iter()
            .find(|node| node.node_id == "agent")
            .expect("completed node");
        assert_eq!(
            node_payload(&node)["completion"]["warnings"],
            serde_json::json!(["file_check_unavailable"])
        );
    });
}

/// Unrelated oversized contents must not disable a usable mustChange baseline for a small file.
#[test]
fn must_change_stays_enforced_when_an_unrelated_file_exceeds_the_snapshot_budget() {
    run_test(async {
        let fixture = Fixture::awaiting_on_git_graph(COMPLETION_MUST_CHANGE_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join("reports/keep.md"), "unchanged\n").expect("deliverable");
        });
        fs::File::create(fixture.workspace.join("oversized-unrelated.bin"))
            .expect("unrelated file")
            .set_len(65 * 1024 * 1024)
            .expect("oversized sibling");
        fixture.settle_turn();
        assert_eq!(
            incomplete_blockers(
                fixture
                    .complete(false)
                    .await
                    .expect_err("unrelated content cannot waive a usable file change requirement")
            ),
            vec![WorkflowNodeCompletionBlocker {
                kind: WorkflowNodeCompletionBlockerKind::FileUnchanged,
                message: None,
                params: Some(WorkflowNodeCompletionBlockerParams {
                    path: Some("reports/keep.md".to_string()),
                }),
            }]
        );
        fs::write(fixture.workspace.join("reports/keep.md"), "changed\n")
            .expect("change deliverable");
        assert_eq!(
            fixture
                .complete(false)
                .await
                .expect("changed file completes")
                .run
                .status,
            WorkflowRunStatus::Succeeded
        );
    });
}

/// An ignored output that existed at node start still has a meaningful mustChange requirement.
#[test]
fn must_change_stays_enforced_for_a_declared_gitignored_file() {
    run_test(async {
        let fixture = Fixture::awaiting_on_git_graph(COMPLETION_MUST_CHANGE_GRAPH, |workspace| {
            fs::create_dir_all(workspace.join("reports")).expect("reports directory");
            fs::write(workspace.join(".gitignore"), "reports/\n").expect("ignore outputs");
            fs::write(workspace.join("reports/keep.md"), "unchanged\n").expect("deliverable");
        });
        fixture.settle_turn();
        assert_eq!(
            incomplete_blockers(
                fixture.complete(false).await.expect_err(
                    "a declared existing artifact must change even when Git ignores it"
                )
            ),
            vec![WorkflowNodeCompletionBlocker {
                kind: WorkflowNodeCompletionBlockerKind::FileUnchanged,
                message: None,
                params: Some(WorkflowNodeCompletionBlockerParams {
                    path: Some("reports/keep.md".to_string()),
                }),
            }]
        );
        fs::write(fixture.workspace.join("reports/keep.md"), "changed\n")
            .expect("change deliverable");
        assert_eq!(
            fixture
                .complete(false)
                .await
                .expect("changed ignored file completes")
                .run
                .status,
            WorkflowRunStatus::Succeeded
        );
    });
}
