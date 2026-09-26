#![allow(clippy::unwrap_used)]
//! WebSocket transport through the production connector and acceptor, with raw peers standing in
//! for routers where the test needs to observe or forge wire behavior.
use futures_util::{SinkExt, StreamExt};
use ora_node_protocol::{MAX_FRAME_LENGTH, NODE_MESSAGE_FRAME_TYPE};
use ora_node_transport::{
    Acceptor, CloseReason, ConnectFailure, FrameReceiver, FrameSender, TransportError,
    close_connection,
    websocket::{WsAcceptor, WsEndpoint, connect},
};
use pretty_assertions::assert_eq;
use std::{collections::BTreeMap, net::SocketAddr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_tungstenite::tungstenite::{
    Message,
    protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
};

const PATH: &str = "/ora-node/v1";

/// Decides whether the Node's receive result is what a given peer message must produce.
type Expectation = fn(Result<Option<Vec<u8>>, TransportError>) -> bool;

/// Endpoint for a local listener with an optional routing header.
fn endpoint(address: SocketAddr, path: &str, headers: &[(&str, &str)]) -> WsEndpoint {
    WsEndpoint {
        url: format!("ws://{address}{path}"),
        headers: headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect::<BTreeMap<_, _>>(),
    }
}

/// Binds a Node-side acceptor on an ephemeral loopback port.
async fn acceptor() -> (WsAcceptor, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let acceptor = WsAcceptor::new(listener, PATH);
    let address = acceptor.local_addr().unwrap();
    (acceptor, address)
}

/// One binary message carries exactly one frame body in each direction.
#[tokio::test]
async fn frames_round_trip_as_binary_messages() {
    let (acceptor, address) = acceptor().await;
    let body = vec![NODE_MESSAGE_FRAME_TYPE, b'{', b'}'];
    let server = async {
        let (mut receiver, mut sender) = acceptor
            .open(acceptor.accept().await.unwrap())
            .await
            .unwrap();
        let frame = receiver.recv().await.unwrap().unwrap();
        sender.send(frame).await.unwrap();
    };
    let client = async {
        let (mut receiver, mut sender) = connect(&endpoint(address, PATH, &[])).await.unwrap();
        sender.send(body.clone()).await.unwrap();
        receiver.recv().await.unwrap()
    };
    let ((), echoed) = tokio::join!(server, client);
    assert_eq!(echoed, Some(body));
}

/// Configured headers reach the router verbatim; the transport does not interpret them.
#[tokio::test]
async fn configured_headers_are_sent_verbatim() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut seen = None;
        // tungstenite's handshake callback signature fixes the refusal type.
        #[allow(clippy::result_large_err)]
        let capture = |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                       response| {
            seen = Some((
                request.uri().to_string(),
                request.headers()["ate-target-actor"]
                    .to_str()
                    .unwrap()
                    .to_owned(),
            ));
            Ok(response)
        };
        let _stream = tokio_tungstenite::accept_hdr_async(stream, capture)
            .await
            .unwrap();
        seen
    };
    let target = endpoint(
        address,
        "/ora-node/v1?resume=1",
        &[("ate-target-actor", "local/demo")],
    );
    let client = connect(&target);
    let (seen, connected) = tokio::join!(router, client);
    connected.unwrap();
    assert_eq!(
        seen,
        Some(("/ora-node/v1?resume=1".to_owned(), "local/demo".to_owned()))
    );
}

/// Router HTTP refusals, missing listeners, wrong paths and malformed endpoints are classified.
#[tokio::test]
async fn connect_failures_are_classified() {
    for (status, expected) in [
        ("404 Not Found", ConnectFailure::NotFound),
        ("502 Bad Gateway", ConnectFailure::Unreachable),
        ("504 Gateway Timeout", ConnectFailure::Unreachable),
        ("403 Forbidden", ConnectFailure::Rejected),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes())
                .await
                .unwrap();
        };
        let target = endpoint(address, PATH, &[]);
        let (_, result) = tokio::join!(router, connect(&target));
        assert_eq!(result.err().unwrap().failure, expected, "{status}");
    }

    let (acceptor, address) = acceptor().await;
    let server = async {
        let pending = acceptor.accept().await.unwrap();
        assert!(acceptor.open(pending).await.is_err());
    };
    let target = endpoint(address, "/other", &[]);
    let (_, wrong_path) = tokio::join!(server, connect(&target));
    assert_eq!(wrong_path.err().unwrap().failure, ConnectFailure::NotFound);
    drop(acceptor);

    let closed = connect(&endpoint(address, PATH, &[])).await;
    assert_eq!(closed.err().unwrap().failure, ConnectFailure::Unreachable);

    for invalid in [
        WsEndpoint {
            url: "http://127.0.0.1/ora-node/v1".into(),
            headers: BTreeMap::new(),
        },
        endpoint(address, PATH, &[("bad header", "value")]),
        endpoint(address, PATH, &[("x-ok", "bad\nvalue")]),
    ] {
        assert_eq!(
            invalid.validate().err().unwrap().failure,
            ConnectFailure::Rejected
        );
    }
}

/// A busy Node upgrades and closes with the busy code, which the Controller can recognize.
#[tokio::test]
async fn busy_rejection_is_recognizable_close_code() {
    let (acceptor, address) = acceptor().await;
    let server = async { acceptor.reject_busy(acceptor.accept().await.unwrap()).await };
    let client = async {
        let (mut receiver, _sender) = connect(&endpoint(address, PATH, &[])).await.unwrap();
        receiver.recv().await
    };
    let ((), result) = tokio::join!(server, client);
    assert!(result.unwrap_err().is_busy());
}

/// A deliberate close reaches the peer with its reason's code, and the receiving half answers the
/// close while the peer is still connected, so neither side waits out a close-handshake timeout.
#[tokio::test]
async fn deliberate_close_carries_reason_and_is_answered() {
    for reason in [
        CloseReason::Shutdown,
        CloseReason::ProtocolViolation,
        CloseReason::IdentityMismatch,
        CloseReason::PeerSilent,
        CloseReason::InternalError,
    ] {
        let (acceptor, address) = acceptor().await;
        let (answered, keep_open) = tokio::sync::oneshot::channel::<()>();
        let server = async {
            let (mut receiver, mut sender) = acceptor
                .open(acceptor.accept().await.unwrap())
                .await
                .unwrap();
            sender.close(reason).await.unwrap();
            // tungstenite echoes the code, so the reply proves the peer flushed its answer.
            let reply =
                tokio::time::timeout(std::time::Duration::from_secs(/*secs*/ 5), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap_err();
            let _ = answered.send(());
            reply
        };
        let client = async {
            let (mut receiver, _sender) = connect(&endpoint(address, PATH, &[])).await.unwrap();
            let result = receiver.recv().await;
            let _ = keep_open.await;
            result
        };
        let (reply, received) = tokio::join!(server, client);
        let received = received.unwrap_err();
        assert!(received.is_closed_for(reason), "{reason:?}: {received:?}");
        assert!(reply.is_closed_for(reason), "{reason:?}: {reply:?}");
    }
}

/// `close_connection` finishes once the peer's close reply arrives, well within any deadline.
#[tokio::test]
async fn close_connection_finishes_with_the_close_handshake() {
    let (acceptor, address) = acceptor().await;
    let server = async {
        let (mut receiver, mut sender) = acceptor
            .open(acceptor.accept().await.unwrap())
            .await
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(/*secs*/ 5),
            close_connection(&mut receiver, &mut sender, CloseReason::Shutdown),
        )
        .await
    };
    let client = async {
        let (mut receiver, _sender) = connect(&endpoint(address, PATH, &[])).await.unwrap();
        assert!(
            receiver
                .recv()
                .await
                .unwrap_err()
                .is_closed_for(CloseReason::Shutdown)
        );
        // Stay connected: only the close reply, not a dropped socket, may end the server's wait.
        std::future::pending::<()>().await
    };
    tokio::select! {
        finished = server => assert!(finished.is_ok(), "close handshake did not finish"),
        () = client => unreachable!("the client stays connected"),
    }
}

/// A normal close ends cleanly, another code is surfaced, and text or oversized messages are
/// protocol errors rather than frames.
#[tokio::test]
async fn close_codes_and_invalid_messages() {
    let cases: Vec<(Message, Expectation)> = vec![
        (
            Message::Close(Some(CloseFrame {
                code: CloseCode::Normal,
                reason: "".into(),
            })),
            |result| matches!(result, Ok(None)),
        ),
        (
            Message::Close(Some(CloseFrame {
                code: CloseCode::Error,
                reason: "Node connection lost".into(),
            })),
            |result| matches!(result, Err(TransportError::Closed { code: 1011, .. })),
        ),
        (Message::Text("{}".into()), |result| {
            matches!(result, Err(TransportError::UnexpectedMessage))
        }),
        (
            Message::Binary(vec![NODE_MESSAGE_FRAME_TYPE; MAX_FRAME_LENGTH + 1].into()),
            |result| matches!(result, Err(TransportError::WebSocket(_))),
        ),
    ];
    for (message, expected) in cases {
        let (acceptor, address) = acceptor().await;
        let server = async {
            let (mut receiver, _sender) = acceptor
                .open(acceptor.accept().await.unwrap())
                .await
                .unwrap();
            receiver.recv().await
        };
        let peer = async {
            let (mut stream, _) = tokio_tungstenite::connect_async_with_config(
                format!("ws://{address}{PATH}"),
                Some(
                    WebSocketConfig::default()
                        .max_message_size(None)
                        .max_frame_size(None),
                ),
                /*disable_nagle*/ true,
            )
            .await
            .unwrap();
            let _ = stream.send(message).await;
            // Keep the connection open until the Node has decided.
            let _ = stream.next().await;
        };
        let (result, ()) = tokio::join!(server, peer);
        assert!(expected(result));
    }
}

/// Dropping a pending receive while a message arrives in slow fragments, interleaved with a ping,
/// neither loses nor splits the frame: the reassembled message is delivered exactly once.
#[tokio::test]
async fn cancelled_receive_keeps_fragmented_message() {
    use tokio_tungstenite::tungstenite::protocol::frame::{
        Frame,
        coding::{Data, OpCode},
    };
    let (acceptor, address) = acceptor().await;
    let body: Vec<u8> = (0..=u8::MAX).cycle().take(4096).collect();
    let fragments: Vec<Vec<u8>> = body.chunks(512).map(<[u8]>::to_vec).collect();
    let server = async {
        let (mut receiver, _sender) = acceptor
            .open(acceptor.accept().await.unwrap())
            .await
            .unwrap();
        let mut cancelled = 0;
        let frame = loop {
            tokio::select! {
                frame = receiver.recv() => break frame.unwrap(),
                () = tokio::time::sleep(std::time::Duration::from_millis(/*millis*/ 1)) => cancelled += 1,
            }
        };
        let end = receiver.recv().await.unwrap();
        (frame, cancelled, end)
    };
    let peer = async {
        let (mut stream, _) = tokio_tungstenite::connect_async(format!("ws://{address}{PATH}"))
            .await
            .unwrap();
        let last = fragments.len() - 1;
        for (index, fragment) in fragments.iter().enumerate() {
            let opcode = if index == 0 {
                OpCode::Data(Data::Binary)
            } else {
                OpCode::Data(Data::Continue)
            };
            stream
                .send(Message::Frame(Frame::message(
                    fragment.clone(),
                    opcode,
                    /*is_final*/ index == last,
                )))
                .await
                .unwrap();
            if index == 2 {
                stream.send(Message::Ping(Vec::new().into())).await.unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(/*millis*/ 5)).await;
        }
        stream
            .send(Message::Close(Some(CloseFrame {
                code: CloseCode::Normal,
                reason: "".into(),
            })))
            .await
            .unwrap();
        while let Some(Ok(_)) = stream.next().await {}
    };
    let ((frame, cancelled, end), ()) = tokio::join!(server, peer);
    assert_eq!(frame, Some(body));
    assert!(cancelled > 0, "the receive was never cancelled mid-message");
    assert_eq!(end, None);
}
