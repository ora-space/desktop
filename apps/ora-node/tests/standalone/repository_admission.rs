//! Clone Git runs on the executor, so the worker keeps answering while a clone waits on its remote.
//!
//! Evidence for `specs/test-cases/node/repository/admission-and-git-execution.md`.
use super::*;
use crate::support::until;
use ora_node_db::NodeDatabase;
use pretty_assertions::assert_eq;
use std::time::{Duration, Instant};
use tokio::{net::UnixStream, time::timeout};

/// The session's admission deadline; the paused Git outlasts it several times over.
const FRAME_TIMEOUT: Duration = Duration::from_millis(/*millis*/ 2000);

/// Far below the admission deadline, so passing proves the reply never queued behind Git.
const PROMPT: Duration = Duration::from_millis(/*millis*/ 500);

/// Asks for the status of `command`.
fn status_query(command: &CloneRepositoryMessage) -> ControllerToNodeMessage {
    ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command.operation_id.clone(),
        execution_id: command.execution_id.clone(),
        payload: GetExecutionStatus {
            node_id: NodeId::new("test-node"),
        },
    })
}

/// Sends `message` and returns the state the Node reports for `execution`, skipping heartbeats
/// and replayed results; a closed session or a slow reply fails the test.
async fn ask(
    stream: &mut UnixStream,
    message: ControllerToNodeMessage,
    execution: &ExecutionId,
    bound: Duration,
) -> ExecutionState {
    write_controller_message(stream, &message).await.unwrap();
    timeout(bound, async {
        loop {
            match read_node_message(stream).await.unwrap() {
                Some(NodeToControllerMessage::ExecutionStatus(status))
                    if status.execution_id == *execution =>
                {
                    return status.payload.state;
                }
                Some(_) => {}
                None => panic!("session closed while clone Git was paused"),
            }
        }
    })
    .await
    .expect("admission reply waited behind clone Git")
}

/// While one clone's Git is paused past the admission deadline, queries and a new clone are
/// answered promptly on the same session, the second clone waits its turn, and after Git resumes
/// each execution has exactly one Run even though many recovery passes ran meanwhile.
#[test]
fn paused_clone_git_leaves_admission_prompt_and_dispatches_each_clone_once() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        fixture.git(&["update-server-info"]);
        let server = HttpsRepository::new(fixture.path(), fixture.path().join("main").join(".git"));
        server.paused.store(true, Ordering::SeqCst);
        let config = configuration(&fixture, &server);
        let endpoint = fixture.config().home_directory.join("control.sock");
        let mut child = ipc::launch_with_deadline(
            &fixture,
            &config,
            /*frame_timeout_ms*/ FRAME_TIMEOUT.as_millis() as u64,
        );
        until(|| {
            fs::read_to_string(fixture.path().join("ipc.log"))
                .unwrap_or_default()
                .contains("Node IPC listening")
        });
        let first = request(&server, "paused", "main");
        let second = request(&server, "waiting", "main");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let mut stream = ipc::connect(&endpoint, "owner").await;
                assert!(matches!(
                    read_node_message(&mut stream).await.unwrap(),
                    Some(NodeToControllerMessage::HelloAccepted(_))
                ));
                assert_eq!(
                    ask(
                        &mut stream,
                        ControllerToNodeMessage::CloneRepository(first.clone()),
                        &first.execution_id,
                        PROMPT,
                    )
                    .await,
                    ExecutionState::Accepted
                );
                // Running means the attempt is durable and its Git is waiting on the paused remote.
                while ask(
                    &mut stream,
                    status_query(&first),
                    &first.execution_id,
                    PROMPT,
                )
                .await
                    != ExecutionState::Running
                {
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 50)).await;
                }
                let paused_since = Instant::now();
                while paused_since.elapsed() < FRAME_TIMEOUT * 2 {
                    assert_eq!(
                        ask(
                            &mut stream,
                            status_query(&first),
                            &first.execution_id,
                            PROMPT
                        )
                        .await,
                        ExecutionState::Running
                    );
                    tokio::time::sleep(Duration::from_millis(/*millis*/ 200)).await;
                }
                assert_eq!(
                    ask(
                        &mut stream,
                        ControllerToNodeMessage::CloneRepository(second.clone()),
                        &second.execution_id,
                        PROMPT,
                    )
                    .await,
                    ExecutionState::Accepted
                );
                // The executor runs one clone at a time: the second stays reserved while the first
                // still holds it, which a recovery pass must not change.
                tokio::time::sleep(Duration::from_millis(/*millis*/ 500)).await;
                assert_eq!(
                    ask(
                        &mut stream,
                        status_query(&second),
                        &second.execution_id,
                        PROMPT
                    )
                    .await,
                    ExecutionState::Accepted
                );
                server.paused.store(false, Ordering::SeqCst);
                let mut ready = Vec::new();
                // The Node's read deadline is also the Controller's liveness deadline, so keep
                // polling like a Controller would while waiting for the results.
                let mut poll = tokio::time::interval(Duration::from_millis(/*millis*/ 500));
                timeout(Duration::from_secs(/*secs*/ 60), async {
                    while ready.len() < 2 {
                        let message = tokio::select! {
                            _ = poll.tick() => {
                                write_controller_message(&mut stream, &status_query(&first))
                                    .await
                                    .unwrap();
                                continue;
                            }
                            message = read_node_message(&mut stream) => message,
                        };
                        match message.unwrap() {
                            Some(NodeToControllerMessage::CloneResult(event)) => {
                                assert!(
                                    matches!(event.payload, CloneExecutionResult::CloneReady(_)),
                                    "clone did not succeed: {event:?}"
                                );
                                if !ready.contains(&event.execution_id) {
                                    ready.push(event.execution_id);
                                }
                            }
                            Some(_) => {}
                            None => panic!("session closed before both clones completed"),
                        }
                    }
                })
                .await
                .expect("both clones complete once Git resumes");
                assert_eq!(
                    ready,
                    vec![first.execution_id.clone(), second.execution_id.clone()]
                );
            });
        child.terminate();
        let database = NodeDatabase::open(
            &fixture.config().home_directory.join("ora-node.sqlite3"),
            fixture.config().identity,
        )
        .unwrap();
        let journal = database.process_journal().unwrap();
        for command in [&first, &second] {
            assert_eq!(
                (
                    journal.attempts(&command.execution_id).unwrap().len(),
                    journal.pending(&command.execution_id).unwrap().len()
                ),
                (1, 0),
                "{} must have one dispatched, cleaned Run",
                command.execution_id.as_str()
            );
        }
    });
}
