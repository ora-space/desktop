use super::WorkflowRuns;
use crate::workflow::run::test_fixture::{AGENT_GRAPH, bind_and_park, run_test, started_run};
use crate::{Backend, test_backend::backend_paths};
use agent_client_protocol_schema::v1::{ContentBlock, ContentChunk, SessionUpdate, TextContent};
use ora_application::{SessionRepository, WorkflowRunEngineRepository};
use ora_contracts::*;
use ora_db::{SqliteSessionRepository, SqliteWorkflowRunEngineRepository};
use ora_domain::{SessionId, WorkflowNodeRunId};
use ora_history::{HistoryLine, HistoryRecord, history_path};
use pretty_assertions::assert_eq;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

struct Fixture {
    temporary: TempDir,
    backend: Backend,
    runs: Arc<WorkflowRuns>,
    node_run_id: String,
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
}

/// Concurrent public completions commit once, retain the final assistant output, and clean up
/// the owned baseline; a later prompt cannot reopen the terminal workflow node.
#[test]
fn concurrent_completions_commit_once_and_leave_the_session_read_only() {
    run_test(async {
        let fixture = Fixture::awaiting();
        let history = history_path(&fixture.runs.sessions_root, "session-1").expect("history path");
        fs::create_dir_all(history.parent().expect("history parent")).expect("history directory");
        let line = HistoryLine::new(
            "2026-09-06T12:00:00+08:00",
            0,
            HistoryRecord::Update {
                update: Box::new(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new("Final answer".to_string())),
                ))),
            },
        );
        fs::write(
            &history,
            format!("{}\n", serde_json::to_string(&line).expect("history JSON")),
        )
        .expect("settled history");
        fs::create_dir_all(&fixture.runs.baselines_root).expect("baseline directory");
        let baseline = fixture
            .runs
            .baselines_root
            .join(format!("{}.json", fixture.node_run_id));
        fs::write(&baseline, "{}").expect("worktree baseline");
        let request = CompleteWorkflowNodeRequest {
            run_id: "run-1".to_string(),
            node_id: "agent".to_string(),
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
        let completed = fixture
            .runs
            .complete_node(request)
            .await
            .expect("claim released for retry");
        assert_eq!(completed.run.status, WorkflowRunStatus::Succeeded);
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
        let completed = fixture
            .runs
            .complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
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
        let (cancelled, completed) = tokio::join!(
            fixture.runs.cancel(CancelWorkflowRunRequest {
                run_id: "run-1".to_string()
            }),
            fixture.runs.complete_node(CompleteWorkflowNodeRequest {
                run_id: "run-1".to_string(),
                node_id: "agent".to_string(),
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
