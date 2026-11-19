//! Turn cancels with a caller deadline, through the production actor and a fake provider.
//!
//! Evidence for specs/test-cases/node/protocol/cancel-turn-and-keep-session.md cases 24 and 25.
use super::*;
use agent_client_protocol_schema::v1::MessageId;
use pretty_assertions::assert_eq;
use tokio::time::Instant;

/// Case 24: a cancel that arrives after admission committed but before the prompt is written
/// keeps the prompt from the Agent, ends the turn cancelled, and leaves the session reusable.
#[test]
fn a_cancel_before_the_prompt_write_never_reaches_the_agent() {
    run_case(async {
        let mut fixture = Fixture::new().await;
        fixture.add_session("unsent");
        let manager = fixture.manager.clone();
        let mut stream = fixture
            .manager
            .prompt_session_with_admission(prompt("unsent"), MessageId::new("turn-1"), move || {
                // The commit point is the last moment before the write; the cancel lands there.
                manager.cancel_turn_prompt("unsent", Instant::now() + Duration::from_secs(15))
            })
            .await
            .unwrap();
        assert_eq!(completion(&mut stream).await, StopReason::Cancelled);
        assert!(stream.end_report().is_some_and(|report| report.unsent()));
        assert!(fixture.frames.try_recv().is_err(), "no prompt was written");
        drop(stream);
        let mut next = fixture
            .manager
            .prompt_session(prompt("unsent"))
            .await
            .unwrap();
        assert_eq!(fixture.next_frame().await["method"], "session/prompt");
        fixture
            .agent
            .send(AgentInput::Finish("unsent".into(), StopReason::EndTurn))
            .unwrap();
        assert_eq!(completion(&mut next).await, StopReason::EndTurn);
        fixture.shutdown().await;
    });
}

/// Case 25: a turn cancel merged into a watchdog stop replaces the watchdog's short grace with its
/// own deadline, once; the prompt is never resent, and the deadline decides the outcome.
#[test]
fn a_cancel_merged_into_a_watchdog_stop_owns_its_deadline() {
    run_case(async {
        for answered in [true, false] {
            let mut fixture = Fixture::new().await;
            fixture.add_session("default");
            let (configured, confirmed) = oneshot::channel();
            fixture
                .agent
                .send(AgentInput::CancelReply {
                    session_id: "default".to_string(),
                    reply: CancelReply::Ignore,
                    configured,
                })
                .unwrap();
            confirmed.await.unwrap();
            let mut stream = fixture
                .manager
                .prompt_session(prompt("default"))
                .await
                .unwrap();
            assert_eq!(fixture.next_frame().await["method"], "session/prompt");
            tokio::time::advance(Duration::from_secs(45)).await;
            assert_eq!(fixture.next_frame().await["method"], "session/cancel");
            let deadline = Instant::now() + Duration::from_secs(15);
            fixture
                .manager
                .cancel_turn_prompt("default", deadline)
                .unwrap();
            // A later cancel never moves the deadline the first one fixed.
            fixture
                .manager
                .cancel_turn_prompt("default", deadline + Duration::from_secs(60))
                .unwrap();
            // Past the watchdog's own grace, the merged cancel still waits.
            tokio::time::advance(Duration::from_secs(10)).await;
            tokio::task::yield_now().await;
            assert!(stream.try_recv().is_none());
            if answered {
                fixture
                    .agent
                    .send(AgentInput::Finish("default".into(), StopReason::Cancelled))
                    .unwrap();
                assert_eq!(completion(&mut stream).await, StopReason::Cancelled);
                assert_eq!(stream.end_cause(), Some(PromptEndCause::UserStopped));
            } else {
                tokio::time::advance(Duration::from_secs(6)).await;
                let error = loop {
                    match stream.recv().await.expect("the prompt ends") {
                        Err(error) => break error,
                        Ok(_) => {}
                    }
                };
                assert!(error.is_turn_cancel_unanswered());
            }
            assert!(
                fixture.frames.try_recv().is_err(),
                "the prompt is never resent"
            );
            fixture.shutdown().await;
        }
    });
}
