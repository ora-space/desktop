use agent_client_protocol_schema::v1::SessionId;
use agent_client_protocol_schema::v1::SessionNotification;
use agent_client_protocol_schema::v1::{SessionInfoUpdate, SessionUpdate};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::error::AcpError;
use crate::events::{AcpInboundEvent, ExtensionNotification};
use crate::peer::AcpPeer;
use crate::transport::{AcpMessages, AcpTransport};

/// Carries ACP messages as already-parsed values, standing in for the plugin IPC transport.
struct MemoryTransport {
    sent: mpsc::UnboundedSender<Value>,
}

impl AcpTransport for MemoryTransport {
    async fn send(&self, message: Value) -> Result<(), AcpError> {
        let _ = self.sent.send(message);
        Ok(())
    }
}

/// Drives the agent end of one connection: it observes what Ora sent and replies with frames.
///
/// Dropping it ends the inbound stream, which is how these tests reproduce an agent that went
/// away without saying so.
struct AgentLink {
    inbound: mpsc::UnboundedSender<Result<Value, AcpError>>,
    outbound: mpsc::UnboundedReceiver<Value>,
}

impl AgentLink {
    /// Delivers one already-parsed frame to Ora in transport order.
    fn send(&self, frame: Value) {
        self.inbound.send(Ok(frame)).expect("queue inbound frame");
    }

    /// Waits for the next frame Ora wrote, so a reply can correlate on its request id.
    async fn next_outbound(&mut self) -> Value {
        self.outbound.recv().await.expect("receive outbound frame")
    }
}

/// Pairs one peer with the agent end of its connection.
fn spawn_peer() -> (AcpPeer<MemoryTransport>, AgentLink) {
    let (inbound, messages) = mpsc::unbounded_channel();
    let (sent, outbound) = mpsc::unbounded_channel();
    let peer = AcpPeer::spawn(messages as AcpMessages, MemoryTransport { sent });
    (peer, AgentLink { inbound, outbound })
}

/// Builds one session update notification with a stable title.
fn update_frame(session_id: &str, title: &str) -> Value {
    let session_id = session_id.to_string();
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": SessionNotification::new(
            session_id,
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title(title)),
        ),
    })
}

/// Builds one successful response frame correlated to an earlier request.
fn response_frame(request_id: &Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "result": result,
    })
}

/// Returns the update carried by one inbound event, failing every other shape.
fn expect_update(event: AcpInboundEvent) -> SessionNotification {
    match event {
        AcpInboundEvent::SessionUpdate(update) => update,
        AcpInboundEvent::ExtensionNotification(_)
        | AcpInboundEvent::PermissionRequest(_)
        | AcpInboundEvent::SessionResponse(_)
        | AcpInboundEvent::Fatal(_) => panic!("expected session update"),
    }
}

/// Starts one session request and returns it together with the frame Ora wrote for it.
async fn start_prompt(
    peer: &AcpPeer<MemoryTransport>,
    link: &mut AgentLink,
    session_id: &SessionId,
) -> (crate::client::PendingSessionRequest<Value>, Value) {
    let pending = peer
        .client
        .start_session_request::<_, Value>(
            session_id.clone(),
            "session/prompt",
            &json!({ "sessionId": session_id }),
        )
        .await
        .expect("start session request");
    let outbound = link.next_outbound().await;
    (pending, outbound)
}

/// Verifies connection-wide handoff cannot make one burst terminate unrelated sessions.
#[tokio::test]
async fn hands_off_more_than_one_session_queue_of_updates() {
    let (mut peer, link) = spawn_peer();
    let expected = (0..300)
        .map(|index| {
            SessionNotification::new(
                format!("session-{}", index % 2),
                SessionUpdate::SessionInfoUpdate(
                    SessionInfoUpdate::new().title(format!("Update {index}")),
                ),
            )
        })
        .collect::<Vec<_>>();

    for notification in &expected {
        link.send(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": notification,
        }));
    }

    let mut received = Vec::with_capacity(expected.len());
    for _ in 0..expected.len() {
        received.push(expect_update(
            peer.next_event().await.expect("receive session event"),
        ));
    }
    assert_eq!(received, expected);
}

/// Verifies tail updates and their terminating response preserve transport order.
#[tokio::test]
async fn orders_session_updates_before_the_session_response() {
    let (mut peer, mut link) = spawn_peer();
    let session_id = SessionId::new("session-1");
    let (pending, outbound) = start_prompt(&peer, &mut link, &session_id).await;
    let expected = ["First", "Second"].map(|title| {
        SessionNotification::new(
            "session-1",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title(title)),
        )
    });
    for title in ["First", "Second"] {
        link.send(update_frame("session-1", title));
    }
    link.send(response_frame(
        &outbound["id"],
        json!({ "stopReason": "end_turn" }),
    ));

    for expected_update in expected {
        assert_eq!(
            expect_update(peer.next_event().await.expect("receive update event")),
            expected_update
        );
    }
    let response = match peer.next_event().await.expect("receive response event") {
        AcpInboundEvent::SessionResponse(response) => response,
        AcpInboundEvent::SessionUpdate(_)
        | AcpInboundEvent::ExtensionNotification(_)
        | AcpInboundEvent::PermissionRequest(_)
        | AcpInboundEvent::Fatal(_) => panic!("expected session response"),
    };
    assert_eq!(
        pending.finish(response).expect("finish session request"),
        json!({ "stopReason": "end_turn" })
    );
}

/// Verifies abandoning a session request discards its late response without a fatal.
#[tokio::test]
async fn discards_a_late_response_after_the_session_request_is_abandoned() {
    let (mut peer, mut link) = spawn_peer();
    let session_id = SessionId::new("session-1");
    let (pending, outbound) = start_prompt(&peer, &mut link, &session_id).await;
    pending.abandon();
    link.send(response_frame(
        &outbound["id"],
        json!({ "stopReason": "end_turn" }),
    ));
    link.send(update_frame("session-1", "Alive"));

    assert_eq!(
        expect_update(peer.next_event().await.expect("receive follow-up update")),
        SessionNotification::new(
            "session-1",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Alive")),
        )
    );
}

/// Verifies dropping an unsettled handle unregisters the request like an explicit abandon.
#[tokio::test]
async fn dropping_a_pending_session_request_unregisters_it() {
    let (mut peer, mut link) = spawn_peer();
    let session_id = SessionId::new("session-1");
    let (pending, outbound) = start_prompt(&peer, &mut link, &session_id).await;
    drop(pending);
    link.send(response_frame(
        &outbound["id"],
        json!({ "stopReason": "cancelled" }),
    ));
    link.send(update_frame("session-1", "Still open"));

    assert_eq!(
        expect_update(peer.next_event().await.expect("receive follow-up update")),
        SessionNotification::new(
            "session-1",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Still open")),
        )
    );
}

/// Verifies cancelling a direct request retires its id and keeps later traffic readable.
#[tokio::test]
async fn dropping_a_direct_request_future_unregisters_it() {
    let (mut peer, mut link) = spawn_peer();
    let client = peer.client.clone();
    let request = tokio::spawn(async move {
        client
            .request::<_, Value>("session/list", &json!({ "cwd": "/workspace" }))
            .await
    });

    let outbound = link.next_outbound().await;
    request.abort();
    let _ = request.await;

    link.send(response_frame(&outbound["id"], json!({ "sessions": [] })));
    link.send(update_frame("session-1", "Still connected"));

    assert_eq!(
        expect_update(peer.next_event().await.expect("receive follow-up update")),
        SessionNotification::new(
            "session-1",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Still connected")),
        )
    );
}

/// Verifies a response id that was never pending remains a fatal correlation failure.
#[tokio::test]
async fn rejects_a_response_with_an_unknown_id() {
    let (mut peer, mut link) = spawn_peer();
    let session_id = SessionId::new("session-1");
    let (_pending, _outbound) = start_prompt(&peer, &mut link, &session_id).await;
    link.send(response_frame(
        &json!(999),
        json!({ "stopReason": "end_turn" }),
    ));

    match peer.next_event().await.expect("receive fatal event") {
        AcpInboundEvent::Fatal(AcpError::InvalidFrame(message)) => {
            assert_eq!(message, "unmatched response id 999");
        }
        AcpInboundEvent::SessionUpdate(_)
        | AcpInboundEvent::ExtensionNotification(_)
        | AcpInboundEvent::PermissionRequest(_)
        | AcpInboundEvent::SessionResponse(_)
        | AcpInboundEvent::Fatal(_) => panic!("expected unmatched response failure"),
    }
}

/// Verifies extension requests receive method-not-found without closing request correlation,
/// including `_`-prefixed extension methods that carry an id.
#[tokio::test]
async fn rejects_unknown_agent_request_and_continues_reading() {
    let (peer, mut link) = spawn_peer();
    let client = peer.client.clone();
    let request = tokio::spawn(async move {
        client
            .request::<_, Value>("initialize", &json!({ "protocolVersion": 1 }))
            .await
    });

    let outbound = link.next_outbound().await;
    link.send(json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "ext/future",
        "params": {},
    }));

    assert_eq!(
        link.next_outbound().await,
        json!({
            "jsonrpc": "2.0",
            "id": 99,
            "error": {
                "code": -32601,
                "message": "method not found: ext/future",
            },
        })
    );

    link.send(json!({
        "jsonrpc": "2.0",
        "id": 100,
        "method": "_subagent/control",
        "params": {},
    }));

    assert_eq!(
        link.next_outbound().await,
        json!({
            "jsonrpc": "2.0",
            "id": 100,
            "error": {
                "code": -32601,
                "message": "method not found: _subagent/control",
            },
        })
    );

    link.send(response_frame(&outbound["id"], json!({ "accepted": true })));
    assert_eq!(
        request
            .await
            .expect("join request")
            .expect("complete request"),
        json!({ "accepted": true })
    );
}

/// Verifies the end of the inbound stream wakes correlated requests instead of leaving them to an
/// outer timeout.
#[tokio::test]
async fn closes_pending_requests_when_the_inbound_stream_ends() {
    let (peer, mut link) = spawn_peer();
    let client = peer.client.clone();
    let request = tokio::spawn(async move {
        client
            .request::<_, Value>("initialize", &json!({ "protocolVersion": 1 }))
            .await
    });
    let _outbound = link.next_outbound().await;

    drop(link);

    assert!(matches!(
        request.await.expect("join request"),
        Err(AcpError::StreamClosed)
    ));
}

/// Verifies `_`-prefixed extension notifications preserve wire order while unknown standard
/// notifications are ignored.
#[tokio::test]
async fn preserves_underscore_extension_notifications_in_wire_order() {
    let (mut peer, mut link) = spawn_peer();
    let session_id = SessionId::new("session-1");
    let (pending, outbound) = start_prompt(&peer, &mut link, &session_id).await;

    link.send(json!({
        "jsonrpc": "2.0",
        "method": "future/unrecognized_notification",
        "params": { "sessionId": "session-1" },
    }));
    let ext_params = json!({
        "sessionId": "session-1",
        "taskId": "task-42",
        "agentType": "Explore",
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "exploring" },
        },
        "timestamp": 1_730_000_000_000_u64,
    });
    link.send(json!({
        "jsonrpc": "2.0",
        "method": "_subagent/update",
        "params": ext_params,
    }));
    link.send(update_frame("session-1", "After subagent"));
    link.send(response_frame(
        &outbound["id"],
        json!({ "stopReason": "end_turn" }),
    ));

    match peer.next_event().await.expect("receive extension event") {
        AcpInboundEvent::ExtensionNotification(extension) => {
            assert_eq!(
                extension,
                ExtensionNotification {
                    method: "_subagent/update".to_string(),
                    params: ext_params,
                }
            );
        }
        other => panic!("expected extension notification, got {other:?}"),
    }
    assert_eq!(
        expect_update(peer.next_event().await.expect("receive session update")),
        SessionNotification::new(
            "session-1",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("After subagent")),
        )
    );
    let response = match peer.next_event().await.expect("receive session response") {
        AcpInboundEvent::SessionResponse(response) => response,
        other => panic!("expected session response, got {other:?}"),
    };
    assert_eq!(
        pending.finish(response).expect("finish session request"),
        json!({ "stopReason": "end_turn" })
    );
}

/// A direct answer is placed among the inbound events in the order its frame was read, so a caller
/// can tell the events the agent sent before it from those it sent after.
#[tokio::test]
async fn a_direct_answer_is_ordered_among_the_events_read_around_it() {
    let (peer, mut link) = spawn_peer();
    let (client, mut inbound) = peer.into_parts();
    let request = tokio::spawn(async move {
        client
            .request_read_in_order::<_, Value>("session/new", &json!({}))
            .await
    });
    let frame = link.next_outbound().await;
    link.send(update_frame("created", "before"));
    link.send(response_frame(
        &frame["id"],
        json!({ "sessionId": "created" }),
    ));
    link.send(update_frame("created", "after"));

    let (answer, answered) = request.await.expect("request task").expect("direct answer");
    let before = inbound.recv().await.expect("event before the answer");
    let after = inbound.recv().await.expect("event after the answer");
    assert_eq!(answer, json!({ "sessionId": "created" }));
    assert!(
        before.order < answered,
        "{before:?} was read before {answered:?}"
    );
    assert!(
        answered < after.order,
        "{after:?} was read after {answered:?}"
    );
    let expected = |title| update_frame("created", title)["params"].clone();
    let received = |event| serde_json::to_value(expect_update(event)).expect("update");
    assert_eq!(received(before.event), expected("before"));
    assert_eq!(received(after.event), expected("after"));
}

/// Prepares one session request on a fresh peer, which numbers it 1, without writing it.
fn prepare_prompt(
    peer: &AcpPeer<MemoryTransport>,
    session_id: &SessionId,
) -> (
    crate::client::PendingSessionRequest<Value>,
    impl std::future::Future<Output = Result<(), AcpError>> + use<>,
) {
    peer.client.prepare_session_request::<_, Value>(
        session_id.clone(),
        "session/prompt",
        &json!({ "sessionId": session_id }),
    )
}

/// A request is registered before its frame is written, so an answer that overtakes the write
/// still reaches the handle in order with the session's events.
#[tokio::test]
async fn an_answer_that_overtakes_its_write_is_ordered_for_the_handle() {
    let (mut peer, mut link) = spawn_peer();
    let session_id = SessionId::new("session-1");
    let (pending, write) = prepare_prompt(&peer, &session_id);

    link.send(response_frame(
        &json!(1),
        json!({ "stopReason": "end_turn" }),
    ));
    let response = match peer.next_event().await.expect("answer event") {
        AcpInboundEvent::SessionResponse(response) => response,
        other => panic!("expected the answer, got {other:?}"),
    };
    write.await.expect("write the request");

    assert_eq!(link.next_outbound().await["id"], 1);
    assert_eq!(
        pending.finish(response).expect("finish session request"),
        json!({ "stopReason": "end_turn" })
    );
}

/// A write that is dropped never reaches the agent, and the handle dropped with it retires the id,
/// so an answer to it anyway is ignored rather than failing the connection.
#[tokio::test]
async fn a_dropped_write_sends_nothing() {
    let (mut peer, mut link) = spawn_peer();
    let session_id = SessionId::new("session-1");
    let (pending, write) = prepare_prompt(&peer, &session_id);
    drop(write);
    drop(pending);

    link.send(response_frame(
        &json!(1),
        json!({ "stopReason": "end_turn" }),
    ));
    link.send(update_frame("session-1", "after"));

    expect_update(peer.next_event().await.expect("later event"));
    assert!(link.outbound.try_recv().is_err());
}

/// A handle dropped after its request was written retires the id: the late answer is ignored and
/// the session's later events still arrive.
#[tokio::test]
async fn a_dropped_handle_ignores_its_late_answer() {
    let (mut peer, mut link) = spawn_peer();
    let session_id = SessionId::new("session-1");
    let (pending, write) = prepare_prompt(&peer, &session_id);
    write.await.expect("write the request");
    let request = link.next_outbound().await;
    drop(pending);

    link.send(response_frame(
        &request["id"],
        json!({ "stopReason": "end_turn" }),
    ));
    link.send(update_frame("session-1", "after"));

    let event = peer.next_event().await.expect("later event");
    assert!(
        matches!(event, AcpInboundEvent::SessionUpdate(_)),
        "{event:?}"
    );
}
