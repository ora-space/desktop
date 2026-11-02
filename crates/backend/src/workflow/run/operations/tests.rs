use super::WorkflowRuns;
use crate::git_cleanup::KeyedResourceLocks;
use crate::workflow::run::test_fixture::{
    AGENT_GRAPH, RecordingExecutor, bind_and_park, bootstrap, run_test, started_run,
    started_run_with, write_open_turn, write_settled_turn,
};
use crate::{Backend, test_backend::backend_paths};
use ora_application::{
    NodeFailure, NodeFailureKind, ResumeWorkflowRunResult, SessionRepository,
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

    /// Writes one settled turn so the completion gate sees a finished turn for the bound session.
    fn settle_turn(&self) {
        write_settled_turn(&self.runs.sessions_root, "session-1", "go", "Final answer");
    }
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
