//! Human turns that end abnormally through the production session prompt path.
//!
//! Each case opens a real interactive workflow, parks its opening turn, then sends a human prompt
//! through `backend.sessions().prompt`. The workflow admission, runtime actor, real history and
//! SQLite rows, and the park cleanup all run; only the fake agent's failure mode is synthetic.
//! The assertions cover the snapshot the UI reads and the default completion gate, not just the
//! runtime's own end report.

use super::abnormal_support::{
    CRASH_MARKER, DROP_MARKER, SILENT_MARKER, open_abnormal_backend, prompt_marker_count,
    wait_for_prompt_marker,
};
use super::workflow_interactive::{
    complete, completion_blockers, completion_snapshot, start_interactive_run, wait_for_node,
};
use super::{
    current_thread_runtime, install_fake_opencode_plugin, open_ready_backend, seed_workspace,
};
use crate::setup::DesktopTestSetup;
use agent_client_protocol_schema::v1::{ContentBlock, TextContent};
use ora_backend::{Backend, BackendError, SessionEventStream};
use ora_contracts::{
    EmptyErrorParams, PromptSessionEvent, PromptSessionRequest, PublicError,
    WorkflowNodeCompletionBlocker, WorkflowNodeCompletionBlockerKind, WorkflowNodeIncompleteParams,
    WorkflowNodeRun, WorkflowNodeStatus,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::time::Duration;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Absolute deadline for one abnormal human-turn case and all of its cleanup waits.
const RUN_DEADLINE: Duration = Duration::from_secs(20);

/// Runs one case on a current-thread runtime with an absolute failure deadline.
fn run_case(test: impl std::future::Future<Output = TestResult>) -> TestResult {
    current_thread_runtime()?.block_on(async {
        tokio::time::timeout(RUN_DEADLINE, test)
            .await
            .map_err(|_| "abnormal human-turn case exceeded its deadline")?
    })
}

/// The opening prompt parks the interactive node before the abnormal human turn starts.
async fn parked_interactive_run(
    backend: &Backend,
    workspace_id: &str,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let run_id = start_interactive_run(backend, workspace_id)?;
    let parked = wait_for_node(backend, &run_id, "agent", |node| {
        node.status == WorkflowNodeStatus::Pending
    })
    .await?;
    let session_id = parked.session_id.ok_or("the parked node owns a session")?;
    Ok((run_id, session_id))
}

/// Drains a prompt stream until it reports the terminal error, counting retry notices.
async fn drain_until_error(
    stream: &mut SessionEventStream<PromptSessionEvent>,
) -> (usize, Option<BackendError>) {
    let mut retries = 0;
    let mut error = None;
    while let Some(event) = stream.recv().await {
        match event {
            Ok(PromptSessionEvent::Retrying { .. }) => retries += 1,
            Ok(_) => {}
            Err(terminal) => {
                error = Some(terminal);
                break;
            }
        }
    }
    (retries, error)
}

/// Waits for the abnormal snapshot the workflow park writes for the human turn.
async fn wait_for_abnormal_park(
    backend: &Backend,
    run_id: &str,
) -> Result<WorkflowNodeRun, Box<dyn std::error::Error>> {
    wait_for_node(backend, run_id, "agent", |node| {
        node.status == WorkflowNodeStatus::Pending && !completion_blockers(node).is_empty()
    })
    .await
}

/// Asserts the abnormal classification and cause recorded in the node's completion snapshot.
fn assert_abnormal_snapshot(node: &WorkflowNodeRun, cause: &str) {
    let snapshot = completion_snapshot(node);
    assert_eq!(
        completion_blockers(node)
            .into_iter()
            .map(|blocker| blocker.kind)
            .collect::<Vec<_>>(),
        vec![WorkflowNodeCompletionBlockerKind::TurnStopped],
        "an abnormal human turn parks as turn_stopped"
    );
    assert_eq!(
        snapshot["parkKind"],
        json!("abnormal"),
        "the snapshot must not present an abnormal end as awaiting the user"
    );
    assert_eq!(
        snapshot["stopCause"],
        json!(cause),
        "the owning prompt's end cause must reach the park snapshot"
    );
    assert_eq!(snapshot["turnOrigin"], json!("human"));
    assert_eq!(snapshot["provisional"], json!(false));
}

/// A normal completion is refused until the user explicitly continues or force-completes.
async fn assert_default_completion_rejected(backend: &Backend, run_id: &str) {
    let error = match complete(backend, run_id).await {
        Ok(()) => panic!("an abnormal turn cannot complete without force"),
        Err(error) => error,
    };
    assert_eq!(
        error.public_error(),
        &PublicError::WorkflowNodeIncomplete(WorkflowNodeIncompleteParams {
            node_id: "agent".to_string(),
            blockers: vec![WorkflowNodeCompletionBlocker {
                kind: WorkflowNodeCompletionBlockerKind::TurnStopped,
                message: None,
                params: None,
            }],
        }),
        "the default completion must name the blocking turn, not advance the run"
    );
}

/// Waits for the opening park, then sends one human prompt carrying `marker`.
async fn send_human_prompt(
    backend: &Backend,
    session_id: &str,
    marker: &str,
) -> Result<SessionEventStream<PromptSessionEvent>, Box<dyn std::error::Error>> {
    Ok(backend
        .sessions()
        .prompt(PromptSessionRequest {
            session_id: session_id.to_string(),
            prompt: vec![ContentBlock::Text(TextContent::new(format!(
                "carry on {marker}"
            )))],
            record_prompt: None,
            model: None,
        })
        .await?)
}

/// The inactivity schedule exhausts every retry and parks the human turn as a system timeout.
#[test]
fn a_timed_out_human_turn_parks_as_abnormal_with_prompt_timed_out() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_abnormal_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let (run_id, session_id) = parked_interactive_run(&backend, &workspace_id).await?;

        let mut stream = send_human_prompt(&backend, &session_id, SILENT_MARKER).await?;
        let (retries, error) = drain_until_error(&mut stream).await;
        drop(stream);
        assert_eq!(
            retries, 3,
            "all three automatic prompt retries must happen, not just one cancellation"
        );
        assert_eq!(
            error
                .as_ref()
                .ok_or("the exhausted inactivity schedule reports its failure")?
                .public_error(),
            &PublicError::AgentTimedOut(EmptyErrorParams {}),
            "the terminal error is the runtime's own agent timeout"
        );

        let parked = wait_for_abnormal_park(&backend, &run_id).await?;
        assert_abnormal_snapshot(&parked, "prompt_timed_out");
        assert_eq!(
            prompt_marker_count(&package_root, SILENT_MARKER),
            4,
            "the provider must receive the initial attempt and every re-sent attempt"
        );
        assert_default_completion_rejected(&backend, &run_id).await;
        Ok(())
    })
}

/// A process crash is reported as a crashed session, not as a stream its owner dropped.
#[test]
fn a_crashed_human_turn_parks_as_abnormal_with_session_crashed() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let (run_id, session_id) = parked_interactive_run(&backend, &workspace_id).await?;

        let mut stream = send_human_prompt(&backend, &session_id, CRASH_MARKER).await?;
        wait_for_prompt_marker(&package_root, CRASH_MARKER).await?;
        let (retries, error) = drain_until_error(&mut stream).await;
        drop(stream);
        assert_eq!(retries, 0, "a crash is not an inactivity retry");
        assert_eq!(
            error
                .as_ref()
                .ok_or("the crashed provider reports its failure")?
                .public_error(),
            &PublicError::AgentRuntimeUnavailable(EmptyErrorParams {}),
            "the prompt stream ends with the runtime-unavailable error"
        );

        let parked = wait_for_abnormal_park(&backend, &run_id).await?;
        assert_abnormal_snapshot(&parked, "session_crashed");
        assert_eq!(prompt_marker_count(&package_root, CRASH_MARKER), 1);
        assert_default_completion_rejected(&backend, &run_id).await;
        Ok(())
    })
}

/// The owner dropping its live prompt stream is distinct from the provider process dying.
#[test]
fn a_dropped_human_prompt_stream_parks_as_abnormal_with_stream_dropped() -> TestResult {
    run_case(async {
        let setup = DesktopTestSetup::new()?;
        let package_root = install_fake_opencode_plugin(&setup.backend_paths().home_directory)?;
        let backend = open_ready_backend(&setup)?;
        let workspace_id = seed_workspace(&setup, &backend)?;
        let (run_id, session_id) = parked_interactive_run(&backend, &workspace_id).await?;

        let mut stream = send_human_prompt(&backend, &session_id, DROP_MARKER).await?;
        wait_for_prompt_marker(&package_root, DROP_MARKER).await?;
        // This is the owning stream abandoning its turn, not the user-cancel endpoint: the
        // runtime must attribute it to the dropped stream while the provider process stays alive.
        stream.cancel_and_wait().await?;
        drop(stream);

        let parked = wait_for_abnormal_park(&backend, &run_id).await?;
        assert_abnormal_snapshot(&parked, "stream_dropped");
        assert_default_completion_rejected(&backend, &run_id).await;
        assert_eq!(
            prompt_marker_count(&package_root, DROP_MARKER),
            1,
            "the drop case must not fall back to a second crash attempt"
        );

        // The plugin survived the dropped stream and can still serve a normal follow-up turn,
        // which is the observable distinction from the crash case above.
        super::workflow_interactive::say(&backend, &session_id, "the process is still alive")
            .await?;
        let recovered = wait_for_node(&backend, &run_id, "agent", |node| {
            node.status == WorkflowNodeStatus::Pending && completion_blockers(node).is_empty()
        })
        .await?;
        assert_eq!(
            completion_snapshot(&recovered)["stopCause"],
            json!(null),
            "a later settled human turn replaces the dropped-stream snapshot"
        );
        Ok(())
    })
}
