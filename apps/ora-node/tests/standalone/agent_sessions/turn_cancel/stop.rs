//! Case 12: cancellation settles even when the original stop proof remains unavailable.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn missing_stop_proof_keeps_cancel_settled_without_a_terminal() {
    ora_logging::with_trace_logging(|| {
        let fixture = Fixture::new();
        let server = GitServerFixture::new(fixture.path(), fixture.path().join("main/.git"));
        let config = configuration(&fixture, &server);
        prepare(&fixture, &config, &server);
        let cloned = checkout(&fixture);
        let mut child = launch_with_stop_bound(&fixture, &config, 500);
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            let (mut stream, _) = greet(&fixture, /*turn_cancel*/ true).await;
            let mut input = start("[ignore-cancel]");
            input.payload.spec.keep_session_on_cancel = true;
            send(&mut stream, ControllerToNodeMessage::StartAgentSession(input)).await;
            loop {
                if matches!(receive(&mut stream).await, NodeToControllerMessage::ThreadEvent(_)) { break; }
            }
            let db = rusqlite::Connection::open(fixture.config().home_directory.join("ora-node.sqlite3")).unwrap();
            let attempt: String = db.query_row("SELECT attempt FROM session_stop_attempts WHERE execution=?1 AND json_extract(record,'$.kind')='agent'", [EXECUTION], |row| row.get(0)).unwrap();
            let lock = fixture.config().home_directory.join("session-attempts").join(format!("{attempt}.lock"));
            let original = fs::read(&lock).unwrap();
            // A malformed original marker is never accepted as affirmative stop proof, even
            // after all processes exited. Restoring it later exercises the reconciliation path.
            fs::write(&lock, b"unavailable original proof").unwrap();
            let began = std::time::Instant::now();
            send(&mut stream, cancel("cancel-proof", "first")).await;
            let settlement = loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::TurnCancelSettled(event) => break event,
                    NodeToControllerMessage::AgentSessionEnded(event) => panic!("early terminal {event:?}"),
                    _ => {}
                }
            };
            assert!(began.elapsed() < std::time::Duration::from_secs(17));
            assert_eq!(settlement.payload.outcome, TurnCancelOutcome::SessionEnding);
            let completed: bool = db.query_row("SELECT state='completed' FROM node_executions WHERE execution=?1", [EXECUTION], |row| row.get(0)).unwrap();
            assert!(!completed, "the conclusion is independent of stop confirmation");
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let state: String = db.query_row("SELECT state FROM session_stop_attempts WHERE attempt=?1", [&attempt], |row| row.get(0)).unwrap();
                    if state == "stop_unconfirmed" { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }).await.unwrap();
            send(&mut stream, query()).await;
            loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::ExecutionStatus(status) => {
                        assert!(serde_json::to_string(&status).unwrap().contains("stop_unconfirmed"));
                        break;
                    }
                    NodeToControllerMessage::AgentSessionEnded(event) => panic!("deadline emitted terminal {event:?}"),
                    _ => {}
                }
            }
            send(&mut stream, ControllerToNodeMessage::DeliverRevision(super::super::delivery::deliver(&cloned.commit, &server))).await;
            loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::RevisionResult(event) => {
                        let RevisionExecutionResult::RevisionFailed(failed) = event.payload else { panic!("unconfirmed execution delivered") };
                        assert_eq!(failed.failure, RevisionFailureCode::SessionNotSettled);
                        break;
                    }
                    NodeToControllerMessage::AgentSessionEnded(event) => panic!("delivery wrote an early terminal {event:?}"),
                    _ => {}
                }
            }
            let mut resumed = start("must not restore");
            resumed.operation_id = OperationId::new("resume-operation");
            resumed.execution_id = ExecutionId::new("resume-execution");
            resumed.payload.spec.prior_revision = Some(PriorRevision {
                revision_id: RevisionId::new("prior"), final_commit: cloned.commit.clone(),
                base_commit: cloned.commit.clone(), git_repository_url: server.url.clone(),
                delivery_ref: DeliveryRef::parse("refs/ora/deliveries/prior").unwrap(),
            });
            send(&mut stream, ControllerToNodeMessage::StartAgentSession(resumed)).await;
            loop {
                if let NodeToControllerMessage::AgentSessionEnded(event) = receive(&mut stream).await {
                    assert_eq!(event.execution_id.as_str(), "resume-execution");
                    let AgentSessionResult::AgentSessionEnded(result) = event.payload;
                    assert_eq!(result.detail.as_deref(), Some("checkout_busy"));
                    break;
                }
            }
            fs::write(&lock, original).unwrap();
            let ended = loop {
                match receive(&mut stream).await {
                    NodeToControllerMessage::AgentSessionEnded(event) => break event,
                    NodeToControllerMessage::TurnCancelSettled(_) => panic!("cancel settled twice"),
                    _ => {}
                }
            };
            let AgentSessionResult::AgentSessionEnded(result) = ended.payload;
            assert_eq!(result.detail.as_deref(), Some("turn_cancel_unanswered"));
            send(&mut stream, ack(ended.sequence.value())).await;
            send(&mut stream, query()).await;
            assert!(matches!(receive(&mut stream).await, NodeToControllerMessage::ExecutionStatus(_)));
        });
        child.terminate();
    });
}
