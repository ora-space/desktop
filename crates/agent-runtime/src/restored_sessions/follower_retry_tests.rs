use super::*;
use ora_contracts::{LoadSessionEvent, LoadSessionRequest};
use ora_history::HistoryRecord;
use pretty_assertions::assert_eq;
use tokio::sync::Notify;

/// Holds each request until the test has observed the matching public lifecycle boundary.
async fn retry_agent(
    allowed: Arc<Notify>,
    mut reader: DuplexStream,
    mut writer: DuplexStream,
    frames: mpsc::UnboundedSender<Value>,
    exited: watch::Sender<bool>,
) {
    let registration = json!({"jsonrpc":"2.0","method":"ora/register",
        "params":{"methods":[],"emits":["agent/acp"]}});
    write_message(&mut writer, &registration).await.unwrap();
    while let Ok(Some(envelope)) = read_message(&mut reader).await {
        if envelope["method"] == "ora/shutdown" {
            break;
        }
        let frame = &envelope["params"];
        frames.send(frame.clone()).unwrap();
        let id = &frame["id"];
        let mut response = match frame["method"].as_str() {
            Some("session/load") => json!({"id":id, "result":{}}),
            Some("session/prompt") if frame["params"]["sessionId"] == "restored" => {
                allowed.notified().await;
                json!({"id":id, "error":{"code":-32000, "message":"stale session"}})
            }
            Some("session/new") => {
                allowed.notified().await;
                json!({"id":id, "result":{"sessionId":"replacement"}})
            }
            Some("session/prompt") => {
                json!({"id":id, "result":PromptResponse::new(StopReason::EndTurn)})
            }
            _ => continue,
        };
        response["jsonrpc"] = json!("2.0");
        write_message(
            &mut writer,
            &json!({"jsonrpc":"2.0","method":"agent/acp","params":response}),
        )
        .await
        .unwrap();
        if frame["method"] == "session/new" {
            // A setup update is forwarded to the prompt owner before its rebuilt prompt is sent.
            let update = SessionNotification::new(
                "replacement",
                SessionUpdate::Plan(agent_client_protocol_schema::v1::Plan::new(Vec::new())),
            );
            write_message(
                &mut writer,
                &json!({"jsonrpc":"2.0","method":"agent/acp",
                "params":{"jsonrpc":"2.0","method":"session/update","params":update}}),
            )
            .await
            .unwrap();
        }
    }
    exited.send_replace(true);
}

/// Admits a real load follower before returning the stale provider error. The replacement
/// handshake is still held when this returns, so the caller can keep or close its owner.
async fn attach_during_first_prompt() -> (
    ReviewDrainFixture,
    Arc<Notify>,
    SessionEventStream<PromptSessionEvent>,
    SessionEventStream<LoadSessionEvent>,
) {
    let allowed = Arc::new(Notify::new());
    let agent_gate = allowed.clone();
    let mut fixture = ReviewDrainFixture::new_with(move |reader, writer, _, frames, exited| {
        retry_agent(agent_gate, reader, writer, frames, exited)
    })
    .await;
    fixture.add_session("restored");
    let owner = fixture
        .manager
        .prompt_session(prompt("restored"))
        .await
        .unwrap();
    assert_eq!(fixture.next_frame().await["method"], "session/load");
    assert_eq!(fixture.next_frame().await["method"], "session/prompt");
    let follower = fixture
        .manager
        .load_session(LoadSessionRequest {
            session_id: "restored".into(),
        })
        .await
        .unwrap();
    allowed.notify_one();
    assert_eq!(fixture.next_frame().await["method"], "session/new");
    (fixture, allowed, owner, follower)
}

/// Checks both live delivery order through EOF and exactly one durable terminal record.
async fn assert_turn_end(
    fixture: &ReviewDrainFixture,
    mut follower: SessionEventStream<LoadSessionEvent>,
    reason: StopReason,
) {
    let mut boundaries = Vec::new();
    while let Some(event) = follower.recv().await {
        match event.unwrap() {
            end @ (LoadSessionEvent::TurnEnded { .. } | LoadSessionEvent::Completed) => {
                boundaries.push(end)
            }
            LoadSessionEvent::SessionUpdate { .. } => {}
            unexpected => panic!("unexpected follower event: {unexpected:?}"),
        }
    }
    assert_eq!(
        boundaries,
        vec![
            LoadSessionEvent::turn_ended(reason),
            LoadSessionEvent::Completed
        ]
    );
    let history =
        ora_history::read_session_history(&fixture.manager.inner.sessions_root, "restored")
            .unwrap();
    let reasons: Vec<_> = history
        .lines
        .into_iter()
        .filter_map(|line| match line.record {
            HistoryRecord::TurnEnded { stop_reason, .. } => Some(stop_reason),
            _ => None,
        })
        .collect();
    assert_eq!(reasons, vec![reason]);
}

/// Losing the owner during rebuild closes the recorded turn before sending a replacement prompt.
#[test]
fn follower_retry_records_cancellation_if_owner_closes_before_resend() {
    run_case(async {
        let (mut fixture, allowed, owner, follower) = attach_during_first_prompt().await;
        drop(owner);
        allowed.notify_one();
        assert_turn_end(&fixture, follower, StopReason::Cancelled).await;
        assert!(
            fixture.frames.try_recv().is_err(),
            "replacement prompt must not be sent"
        );
        fixture.shutdown().await;
    });
}

/// A successful rebuild carries the already admitted follower through the replacement turn.
#[test]
fn follower_retry_survives_rebuild_and_receives_the_completed_turn() {
    run_case(async {
        let (mut fixture, allowed, mut owner, follower) = attach_during_first_prompt().await;
        allowed.notify_one();
        let retry = fixture.next_frame().await;
        assert_eq!(
            (&retry["method"], &retry["params"]["sessionId"]),
            (&json!("session/prompt"), &json!("replacement"))
        );
        assert_eq!(completion(&mut owner).await, StopReason::EndTurn);
        assert_turn_end(&fixture, follower, StopReason::EndTurn).await;
        fixture.shutdown().await;
    });
}
