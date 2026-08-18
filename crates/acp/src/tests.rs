use std::sync::{Arc, Mutex};

use agent_client_protocol_schema::v1::SessionId;
use agent_client_protocol_schema::v1::SessionNotification;
use agent_client_protocol_schema::v1::{SessionInfoUpdate, SessionUpdate};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf, duplex, split,
};
use tokio::sync::mpsc;

use crate::error::AcpError;
use crate::events::AcpInboundEvent;
use crate::peer::AcpPeer;
use crate::transport::{AcpMessages, AcpTransport, NdjsonTransport};

type NdjsonPeer = AcpPeer<NdjsonTransport<WriteHalf<DuplexStream>>>;

/// Starts an NDJSON peer over one duplex connection, matching the stdio agent path.
fn spawn_ndjson_peer(
    reader: ReadHalf<DuplexStream>,
    writer: WriteHalf<DuplexStream>,
) -> NdjsonPeer {
    let (transport, messages) = NdjsonTransport::spawn(reader, writer);
    AcpPeer::spawn(messages, transport)
}

/// Carries ACP messages as already-parsed values, standing in for the plugin IPC transport.
struct MemoryTransport {
    sent: Arc<Mutex<Vec<Value>>>,
}

impl AcpTransport for MemoryTransport {
    async fn send(&self, message: Value) -> Result<(), AcpError> {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(message);
        Ok(())
    }
}

/// Projects inbound events onto comparable data so two transports can be asserted equal.
#[derive(Debug, PartialEq)]
enum ObservedEvent {
    Update(SessionNotification),
    Permission(String),
    Response { session_id: String, result: Value },
    Fatal(String),
}

impl From<AcpInboundEvent> for ObservedEvent {
    fn from(event: AcpInboundEvent) -> Self {
        match event {
            AcpInboundEvent::SessionUpdate(update) => Self::Update(update),
            AcpInboundEvent::PermissionRequest(permission) => {
                Self::Permission(permission.request_id.to_string())
            }
            AcpInboundEvent::SessionResponse(response) => Self::Response {
                session_id: response.session_id().to_string(),
                result: response.response.clone().unwrap_or(Value::Null),
            },
            AcpInboundEvent::Fatal(error) => Self::Fatal(error.to_string()),
        }
    }
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

/// Verifies connection-wide handoff cannot make one burst terminate unrelated sessions.
#[tokio::test]
async fn hands_off_more_than_one_session_queue_of_updates() {
    let (ora_stream, mut agent_stream) = duplex(16 * 1024);
    let (ora_reader, ora_writer) = split(ora_stream);
    let mut peer = spawn_ndjson_peer(ora_reader, ora_writer);
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
        let frame = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": notification,
        });
        agent_stream
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .expect("write session update");
    }

    let mut received = Vec::with_capacity(expected.len());
    for _ in 0..expected.len() {
        match peer.next_event().await.expect("receive session event") {
            AcpInboundEvent::SessionUpdate(update) => received.push(update),
            AcpInboundEvent::PermissionRequest(_)
            | AcpInboundEvent::SessionResponse(_)
            | AcpInboundEvent::Fatal(_) => panic!("expected session update"),
        }
    }
    assert_eq!(received, expected);
}

/// Verifies a byte-stream and an already-parsed transport produce identical inbound events.
#[tokio::test]
async fn produces_identical_events_over_both_transports() {
    let session_id = SessionId::new("session-1");
    let inbound_frames = |request_id: Value| {
        vec![
            update_frame("session-1", "First"),
            update_frame("session-1", "Second"),
            json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "result": { "stopReason": "end_turn" },
            }),
        ]
    };

    let (ora_stream, agent_stream) = duplex(16 * 1024);
    let (ora_reader, ora_writer) = split(ora_stream);
    let (agent_reader, mut agent_writer) = split(agent_stream);
    let mut agent_reader = BufReader::new(agent_reader);
    let mut ndjson_peer = spawn_ndjson_peer(ora_reader, ora_writer);
    let _ndjson_request = ndjson_peer
        .client
        .start_session_request::<_, Value>(
            session_id.clone(),
            "session/prompt",
            &json!({ "sessionId": session_id }),
        )
        .await
        .expect("start session request");
    let mut outbound = String::new();
    agent_reader
        .read_line(&mut outbound)
        .await
        .expect("read session request");
    let ndjson_outbound: Value =
        serde_json::from_str(outbound.trim()).expect("parse session request");
    for frame in inbound_frames(ndjson_outbound["id"].clone()) {
        agent_writer
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .expect("write inbound frame");
    }

    let sent = Arc::new(Mutex::new(Vec::new()));
    let (memory_sender, memory_messages) = mpsc::unbounded_channel();
    let mut memory_peer = AcpPeer::spawn(
        memory_messages as AcpMessages,
        MemoryTransport { sent: sent.clone() },
    );
    let _memory_request = memory_peer
        .client
        .start_session_request::<_, Value>(
            session_id.clone(),
            "session/prompt",
            &json!({ "sessionId": session_id }),
        )
        .await
        .expect("start session request");
    let memory_outbound = sent
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .first()
        .cloned()
        .expect("record session request");
    for frame in inbound_frames(memory_outbound["id"].clone()) {
        memory_sender.send(Ok(frame)).expect("queue inbound frame");
    }

    let mut ndjson_events = Vec::new();
    let mut memory_events = Vec::new();
    for _ in 0..3 {
        ndjson_events.push(ObservedEvent::from(
            ndjson_peer.next_event().await.expect("ndjson event"),
        ));
        memory_events.push(ObservedEvent::from(
            memory_peer.next_event().await.expect("memory event"),
        ));
    }

    assert_eq!(ndjson_outbound, memory_outbound);
    assert_eq!(ndjson_events, memory_events);
}

/// Verifies tail updates and their terminating response preserve transport order.
#[tokio::test]
async fn orders_session_updates_before_the_session_response() {
    let (ora_stream, agent_stream) = duplex(16 * 1024);
    let (ora_reader, ora_writer) = split(ora_stream);
    let (agent_reader, mut agent_writer) = split(agent_stream);
    let mut agent_reader = BufReader::new(agent_reader);
    let mut peer = spawn_ndjson_peer(ora_reader, ora_writer);
    let session_id = SessionId::new("session-1");
    let pending = peer
        .client
        .start_session_request::<_, Value>(
            session_id.clone(),
            "session/prompt",
            &json!({ "sessionId": session_id }),
        )
        .await
        .expect("start session request");
    let mut outbound = String::new();
    agent_reader
        .read_line(&mut outbound)
        .await
        .expect("read session request");
    let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse session request");
    let request_id = outbound["id"].clone();
    let expected = ["First", "Second"].map(|title| {
        SessionNotification::new(
            "session-1",
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title(title)),
        )
    });
    for update in &expected {
        agent_writer
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": update,
                    })
                )
                .as_bytes(),
            )
            .await
            .expect("write session update");
    }
    agent_writer
        .write_all(
            format!(
                "{}\n",
                json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "result": { "stopReason": "end_turn" },
                })
            )
            .as_bytes(),
        )
        .await
        .expect("write session response");

    for expected_update in expected {
        match peer.next_event().await.expect("receive update event") {
            AcpInboundEvent::SessionUpdate(update) => assert_eq!(update, expected_update),
            AcpInboundEvent::PermissionRequest(_)
            | AcpInboundEvent::SessionResponse(_)
            | AcpInboundEvent::Fatal(_) => panic!("expected session update"),
        }
    }
    let response = match peer.next_event().await.expect("receive response event") {
        AcpInboundEvent::SessionResponse(response) => response,
        AcpInboundEvent::SessionUpdate(_)
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
    let (ora_stream, agent_stream) = duplex(16 * 1024);
    let (ora_reader, ora_writer) = split(ora_stream);
    let (agent_reader, mut agent_writer) = split(agent_stream);
    let mut agent_reader = BufReader::new(agent_reader);
    let mut peer = spawn_ndjson_peer(ora_reader, ora_writer);
    let session_id = SessionId::new("session-1");
    let pending = peer
        .client
        .start_session_request::<_, Value>(
            session_id.clone(),
            "session/prompt",
            &json!({ "sessionId": session_id }),
        )
        .await
        .expect("start session request");
    let mut outbound = String::new();
    agent_reader
        .read_line(&mut outbound)
        .await
        .expect("read session request");
    let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse session request");
    pending.abandon();
    agent_writer
        .write_all(
            format!(
                "{}\n",
                json!({
                    "jsonrpc": "2.0",
                    "id": outbound["id"],
                    "result": { "stopReason": "end_turn" },
                })
            )
            .as_bytes(),
        )
        .await
        .expect("write abandoned session response");
    agent_writer
        .write_all(format!("{}\n", update_frame("session-1", "Alive")).as_bytes())
        .await
        .expect("write follow-up update");

    match peer.next_event().await.expect("receive follow-up update") {
        AcpInboundEvent::SessionUpdate(update) => assert_eq!(
            update,
            SessionNotification::new(
                "session-1",
                SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Alive")),
            )
        ),
        AcpInboundEvent::PermissionRequest(_)
        | AcpInboundEvent::SessionResponse(_)
        | AcpInboundEvent::Fatal(_) => {
            panic!("expected session update after abandoned response")
        }
    }
}

/// Verifies dropping an unsettled handle unregisters the request like an explicit abandon.
#[tokio::test]
async fn dropping_a_pending_session_request_unregisters_it() {
    let (ora_stream, agent_stream) = duplex(16 * 1024);
    let (ora_reader, ora_writer) = split(ora_stream);
    let (agent_reader, mut agent_writer) = split(agent_stream);
    let mut agent_reader = BufReader::new(agent_reader);
    let mut peer = spawn_ndjson_peer(ora_reader, ora_writer);
    let session_id = SessionId::new("session-1");
    let pending = peer
        .client
        .start_session_request::<_, Value>(
            session_id.clone(),
            "session/prompt",
            &json!({ "sessionId": session_id }),
        )
        .await
        .expect("start session request");
    let mut outbound = String::new();
    agent_reader
        .read_line(&mut outbound)
        .await
        .expect("read session request");
    let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse session request");
    drop(pending);
    agent_writer
        .write_all(
            format!(
                "{}\n",
                json!({
                    "jsonrpc": "2.0",
                    "id": outbound["id"],
                    "result": { "stopReason": "cancelled" },
                })
            )
            .as_bytes(),
        )
        .await
        .expect("write dropped session response");
    agent_writer
        .write_all(format!("{}\n", update_frame("session-1", "Still open")).as_bytes())
        .await
        .expect("write follow-up update");

    match peer.next_event().await.expect("receive follow-up update") {
        AcpInboundEvent::SessionUpdate(update) => assert_eq!(
            update,
            SessionNotification::new(
                "session-1",
                SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Still open")),
            )
        ),
        AcpInboundEvent::PermissionRequest(_)
        | AcpInboundEvent::SessionResponse(_)
        | AcpInboundEvent::Fatal(_) => panic!("expected session update after dropped request"),
    }
}

/// Verifies cancelling a direct request retires its id and keeps later traffic readable.
#[tokio::test]
async fn dropping_a_direct_request_future_unregisters_it() {
    let (ora_stream, agent_stream) = duplex(16 * 1024);
    let (ora_reader, ora_writer) = split(ora_stream);
    let (agent_reader, mut agent_writer) = split(agent_stream);
    let mut agent_reader = BufReader::new(agent_reader);
    let mut peer = spawn_ndjson_peer(ora_reader, ora_writer);
    let client = peer.client.clone();
    let request = tokio::spawn(async move {
        client
            .request::<_, Value>("session/list", &json!({ "cwd": "/workspace" }))
            .await
    });

    let mut outbound = String::new();
    agent_reader
        .read_line(&mut outbound)
        .await
        .expect("read direct request");
    let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse direct request");
    request.abort();
    let _ = request.await;

    agent_writer
        .write_all(
            format!(
                "{}\n",
                json!({
                    "jsonrpc": "2.0",
                    "id": outbound["id"],
                    "result": { "sessions": [] },
                })
            )
            .as_bytes(),
        )
        .await
        .expect("write abandoned direct response");
    agent_writer
        .write_all(format!("{}\n", update_frame("session-1", "Still connected")).as_bytes())
        .await
        .expect("write follow-up update");

    match peer.next_event().await.expect("receive follow-up update") {
        AcpInboundEvent::SessionUpdate(update) => assert_eq!(
            update,
            SessionNotification::new(
                "session-1",
                SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Still connected")),
            )
        ),
        AcpInboundEvent::PermissionRequest(_)
        | AcpInboundEvent::SessionResponse(_)
        | AcpInboundEvent::Fatal(_) => {
            panic!("expected update after abandoned direct response")
        }
    }
}

/// Verifies a response id that was never pending remains a fatal correlation failure.
#[tokio::test]
async fn rejects_a_response_with_an_unknown_id() {
    let (ora_stream, agent_stream) = duplex(16 * 1024);
    let (ora_reader, ora_writer) = split(ora_stream);
    let (agent_reader, mut agent_writer) = split(agent_stream);
    let mut agent_reader = BufReader::new(agent_reader);
    let mut peer = spawn_ndjson_peer(ora_reader, ora_writer);
    let session_id = SessionId::new("session-1");
    let _pending = peer
        .client
        .start_session_request::<_, Value>(
            session_id.clone(),
            "session/prompt",
            &json!({ "sessionId": session_id }),
        )
        .await
        .expect("start session request");
    let mut outbound = String::new();
    agent_reader
        .read_line(&mut outbound)
        .await
        .expect("read session request");
    agent_writer
        .write_all(
            format!(
                "{}\n",
                json!({
                    "jsonrpc": "2.0",
                    "id": 999,
                    "result": { "stopReason": "end_turn" },
                })
            )
            .as_bytes(),
        )
        .await
        .expect("write unmatched response");

    match peer.next_event().await.expect("receive fatal event") {
        AcpInboundEvent::Fatal(AcpError::InvalidFrame(message)) => {
            assert_eq!(message, "unmatched response id 999");
        }
        AcpInboundEvent::SessionUpdate(_)
        | AcpInboundEvent::PermissionRequest(_)
        | AcpInboundEvent::SessionResponse(_)
        | AcpInboundEvent::Fatal(_) => panic!("expected unmatched response failure"),
    }
}

/// Verifies extension requests receive method-not-found without closing request correlation.
#[tokio::test]
async fn rejects_unknown_agent_request_and_continues_reading() {
    let (ora_stream, agent_stream) = duplex(16 * 1024);
    let (ora_reader, ora_writer) = split(ora_stream);
    let (agent_reader, mut agent_writer) = split(agent_stream);
    let mut agent_reader = BufReader::new(agent_reader);
    let peer = spawn_ndjson_peer(ora_reader, ora_writer);
    let client = peer.client.clone();
    let request = tokio::spawn(async move {
        client
            .request::<_, Value>("initialize", &json!({ "protocolVersion": 1 }))
            .await
    });

    let mut outbound = String::new();
    agent_reader
        .read_line(&mut outbound)
        .await
        .expect("read Ora request");
    let outbound: Value = serde_json::from_str(outbound.trim()).expect("parse Ora request");
    let request_id = outbound["id"].clone();
    agent_writer
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"ext/future\",\"params\":{}}\n")
        .await
        .expect("write extension request");

    let mut rejection = String::new();
    agent_reader
        .read_line(&mut rejection)
        .await
        .expect("read method-not-found response");
    assert_eq!(
        serde_json::from_str::<Value>(rejection.trim()).expect("parse rejection"),
        json!({
            "jsonrpc": "2.0",
            "id": 99,
            "error": {
                "code": -32601,
                "message": "method not found: ext/future",
            },
        })
    );

    let response = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "result": { "accepted": true },
    });
    agent_writer
        .write_all(format!("{response}\n").as_bytes())
        .await
        .expect("write correlated response");
    assert_eq!(
        request
            .await
            .expect("join request")
            .expect("complete request"),
        json!({ "accepted": true })
    );
}

/// Verifies EOF wakes correlated requests instead of leaving them to an outer timeout.
#[tokio::test]
async fn closes_pending_requests_when_agent_stdout_ends() {
    let (ora_stream, agent_stream) = duplex(16 * 1024);
    let (ora_reader, ora_writer) = split(ora_stream);
    let (agent_reader, mut agent_writer) = split(agent_stream);
    let mut agent_reader = BufReader::new(agent_reader);
    let peer = spawn_ndjson_peer(ora_reader, ora_writer);
    let client = peer.client.clone();
    let request = tokio::spawn(async move {
        client
            .request::<_, Value>("initialize", &json!({ "protocolVersion": 1 }))
            .await
    });
    let mut outbound = String::new();
    agent_reader
        .read_line(&mut outbound)
        .await
        .expect("read Ora request");

    agent_writer.shutdown().await.expect("close agent writer");
    drop(agent_reader);
    drop(agent_writer);

    assert!(matches!(
        request.await.expect("join request"),
        Err(AcpError::StreamClosed)
    ));
}
