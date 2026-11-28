//! How the prompt loop attributes a turn that ended without the Agent completing it, observed
//! through the production actor, router and ACP peer: the recorded `TurnEnded` and the error the
//! prompt's owner receives must name the same failure.
//!
//! Evidence for specs/test-cases/node/protocol/turn-failure-reason.md cases 2, 7 and 9.
use super::*;
use ora_history::{HistoryRecord, TurnEndedBy, TurnFailure, TurnFailureCode};
use pretty_assertions::assert_eq;

/// The single `TurnEnded` record the session's history holds.
fn recorded_turn_end(fixture: &Fixture, session: &str) -> HistoryRecord {
    let history =
        ora_history::read_session_history(&fixture._directory.path().join("sessions"), session)
            .unwrap();
    let mut ends: Vec<_> = history
        .lines
        .into_iter()
        .filter_map(|line| match line.record {
            end @ HistoryRecord::TurnEnded { .. } => Some(end),
            _ => None,
        })
        .collect();
    assert_eq!(ends.len(), 1, "one turn ends once");
    ends.remove(0)
}

/// A failure with only its code, as Ora records the Agent's absence and its own errors.
fn bare(code: TurnFailureCode) -> TurnFailure {
    TurnFailure {
        code,
        acp_code: None,
        message: None,
        message_truncated: None,
        attempts: None,
        answer: None,
    }
}

/// A cancelled `TurnEnded` carrying `failure` and no attribution.
fn failed(failure: TurnFailure) -> HistoryRecord {
    HistoryRecord::TurnEnded {
        stop_reason: StopReason::Cancelled,
        ended_by: None,
        failure: Some(failure),
    }
}

/// Starts one prompt on a fresh attached session and returns its stream and the prompt's id.
async fn started(
    fixture: &mut Fixture,
    session: &str,
) -> (SessionEventStream<PromptSessionEvent>, Value) {
    fixture.add_session(session);
    let stream = opened(
        fixture
            .manager
            .prompt_session(prompt(session))
            .await
            .unwrap(),
    )
    .await;
    let frame = fixture.next_frame().await;
    assert_eq!(frame["method"], "session/prompt");
    (stream, frame["id"].clone())
}

/// Reads the stream until its failure.
async fn failure_of(stream: &mut SessionEventStream<PromptSessionEvent>) -> RuntimeError {
    while let Some(event) = stream.recv().await {
        if let Err(error) = event {
            return error;
        }
    }
    panic!("prompt ended without a failure")
}

/// Case 2: ACP's `AuthRequired` code means authentication is required whatever the Agent's text,
/// on the record and on the error the owner maps to a terminal detail.
#[test]
fn the_auth_required_code_needs_authentication_whatever_the_text() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        let (mut stream, id) = started(&mut fixture, "auth").await;
        fixture
            .agent
            .send(AgentInput::RawFrame(json!({
                "jsonrpc":"2.0","id":id,
                "error":{"code":-32000,"message":"Please run the login command"},
            })))
            .unwrap();
        let error = failure_of(&mut stream).await;
        let failure = TurnFailure {
            code: TurnFailureCode::AgentAuthenticationRequired,
            acp_code: Some(-32000),
            message: Some("Please run the login command".into()),
            ..bare(TurnFailureCode::AgentAuthenticationRequired)
        };
        assert_eq!(
            (error.public_error(), error.turn_failure()),
            (
                &PublicError::AgentAuthenticationRequired(EmptyErrorParams {}),
                Some(&failure)
            )
        );
        assert_eq!(recorded_turn_end(&fixture, "auth"), failed(failure));
        fixture.shutdown().await;
    });
}

/// Case 7: a prompt answer that breaks the ACP response shape fails on Ora's side, as `internal`.
#[test]
fn a_malformed_prompt_answer_fails_as_internal() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        let (mut stream, id) = started(&mut fixture, "malformed").await;
        fixture
            .agent
            .send(AgentInput::RawFrame(
                json!({"jsonrpc":"2.0","id":id,"result":{}}),
            ))
            .unwrap();
        let error = failure_of(&mut stream).await;
        let failure = bare(TurnFailureCode::Internal);
        assert_eq!(error.turn_failure(), Some(&failure));
        assert_eq!(recorded_turn_end(&fixture, "malformed"), failed(failure));
        fixture.shutdown().await;
    });
}

/// Case 7: a permission request the runtime cannot answer fails the turn as `internal`.
#[test]
fn an_unanswerable_permission_fails_as_internal() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        let (stream, _) = started(&mut fixture, "unanswerable").await;
        fixture
            .agent
            .send(AgentInput::RawFrame(json!({
                "jsonrpc":"2.0","id":"permission-unanswerable",
                "method":"session/request_permission",
                "params":{
                    "sessionId":"unanswerable","toolCall":{"toolCallId":"tool"},
                    "options":[{"optionId":"no","name":"Reject","kind":"reject_once"}],
                },
            })))
            .unwrap();
        // The turn is stopped once its permission cannot be answered.
        while fixture.next_frame().await["method"] != "session/cancel" {}
        assert_eq!(
            recorded_turn_end(&fixture, "unanswerable"),
            failed(bare(TurnFailureCode::Internal))
        );
        drop(stream);
        fixture.shutdown().await;
    });
}

/// Case 7: the controls queue belongs to the ACP route, not to the host, so seeing it closed
/// first is the same teardown as a lost connection: `agent_unavailable`, never a session end.
#[test]
fn a_closed_control_queue_fails_as_agent_unavailable() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        let (mut stream, _) = started(&mut fixture, "controls").await;
        fixture.supervisor.routes.close_controls("controls");
        let error = failure_of(&mut stream).await;
        let failure = bare(TurnFailureCode::AgentUnavailable);
        assert_eq!(
            error.public_error(),
            &PublicError::AgentRuntimeUnavailable(EmptyErrorParams {})
        );
        assert_eq!(recorded_turn_end(&fixture, "controls"), failed(failure));
        fixture.shutdown().await;
    });
}

/// Case 9: an owner that no longer takes the turn's events is the host ending the session, so
/// the turn is attributed to the session end rather than left without attribution.
#[test]
fn an_owner_that_stops_taking_events_ends_the_turn_as_session_end() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        let (stream, _) = started(&mut fixture, "unread").await;
        let update = || {
            crate::routing::SessionEvent::update(SessionNotification::new(
                "unread",
                SessionUpdate::AgentMessageChunk(
                    agent_client_protocol_schema::v1::ContentChunk::new(ContentBlock::Text(
                        TextContent::new("output nobody reads"),
                    )),
                ),
            ))
        };
        // Fill the owner's queue in batches the actor can forward, without reading any of it.
        for _ in 0..3 {
            for _ in 0..128 {
                fixture.supervisor.routes.route_event(update()).unwrap();
            }
            for _ in 0..16 {
                tokio::task::yield_now().await;
            }
        }
        assert_eq!(fixture.next_frame().await["method"], "session/cancel");
        assert_eq!(
            recorded_turn_end(&fixture, "unread"),
            HistoryRecord::TurnEnded {
                stop_reason: StopReason::Cancelled,
                ended_by: Some(TurnEndedBy::SessionEnd),
                failure: None,
            }
        );
        drop(stream);
        fixture.shutdown().await;
    });
}
