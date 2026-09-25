//! WebSocket transport for Nodes reached through a platform router. One binary message carries
//! exactly one frame body; the message limit equals the protocol frame limit, so the router, the
//! Controller and the Node make the same size decision.
//!
//! WebSocket ping/pong only proves the nearest hop is alive because routers terminate each side;
//! end-to-end liveness remains the protocol heartbeat and the sessions' I/O deadlines.
use crate::{
    Acceptor, CONTROL_SESSION_BUSY_CLOSE_CODE, ConnectError, ConnectFailure, FrameReceiver,
    FrameSender, TransportError,
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use ora_node_protocol::MAX_FRAME_LENGTH;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, future::Future, io, sync::Arc};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{
        Error as WsError, Message,
        client::IntoClientRequest,
        handshake::{
            client::Request,
            server::{ErrorResponse, Response},
        },
        http::{HeaderName, HeaderValue, StatusCode},
        protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
    },
};

/// Where and how to open a WebSocket to one Node. Vendor-specific addressing (for example a
/// routing header naming the sandbox) and platform credentials are plain headers here; this crate
/// sends them verbatim and never interprets them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WsEndpoint {
    /// `ws://` or `wss://` URL including the Node path, such as `/ora-node/v1`.
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

impl WsEndpoint {
    /// Checks everything that can be known without connecting, so deployment errors surface
    /// before any state opens rather than as endless reconnects.
    pub fn validate(&self) -> Result<(), ConnectError> {
        self.request().map(drop)
    }

    /// Builds the upgrade request with the configured headers.
    fn request(&self) -> Result<Request, ConnectError> {
        let rejected = |error: WsError| ConnectError::new(ConnectFailure::Rejected, error);
        let mut request = self.url.as_str().into_client_request().map_err(rejected)?;
        if !matches!(request.uri().scheme_str(), Some("ws" | "wss")) {
            return Err(ConnectError::new(
                ConnectFailure::Rejected,
                "WebSocket endpoint URL must use ws:// or wss://",
            ));
        }
        for (name, value) in &self.headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|error| ConnectError::new(ConnectFailure::Rejected, error))?;
            let value = HeaderValue::from_str(value)
                .map_err(|error| ConnectError::new(ConnectFailure::Rejected, error))?;
            request.headers_mut().insert(name, value);
        }
        Ok(request)
    }
}

/// Message and frame limits both equal the protocol frame limit, so an oversized frame is refused
/// whole instead of being reassembled from fragments.
fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME_LENGTH))
        .max_frame_size(Some(MAX_FRAME_LENGTH))
}

/// Receiving half of a WebSocket connection.
pub struct WsReceiver<S>(SplitStream<WebSocketStream<S>>);

/// Sending half of a WebSocket connection.
pub struct WsSender<S>(SplitSink<WebSocketStream<S>, Message>);

/// Client halves as returned by [`connect`].
pub type ClientReceiver = WsReceiver<MaybeTlsStream<TcpStream>>;
pub type ClientSender = WsSender<MaybeTlsStream<TcpStream>>;

impl<S> FrameReceiver for WsReceiver<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    /// Skips control messages; a normal close is a clean end, any other close code is surfaced so
    /// callers can recognize a busy Node through a router.
    async fn recv(&mut self) -> Result<Option<Vec<u8>>, TransportError> {
        loop {
            let Some(message) = self.0.next().await else {
                return Ok(None);
            };
            match message? {
                Message::Binary(frame) => return Ok(Some(frame.into())),
                Message::Close(None) => return Ok(None),
                Message::Close(Some(frame)) => {
                    if frame.code == CloseCode::Normal {
                        return Ok(None);
                    }
                    return Err(TransportError::Closed {
                        code: frame.code.into(),
                        reason: frame.reason.to_string(),
                    });
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                Message::Text(_) => return Err(TransportError::UnexpectedMessage),
            }
        }
    }
}

impl<S> FrameSender for WsSender<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    /// Sends one frame body as one binary message and flushes it.
    async fn send(&mut self, frame: Vec<u8>) -> Result<(), TransportError> {
        self.0.send(Message::Binary(frame.into())).await?;
        Ok(())
    }
}

/// Splits an upgraded connection into frame halves.
fn split<S>(stream: WebSocketStream<S>) -> (WsReceiver<S>, WsSender<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (sink, stream) = stream.split();
    (WsReceiver(stream), WsSender(sink))
}

/// Opens a WebSocket to a Node, classifying failures by what a router can tell us: unknown
/// sandboxes are `NotFound`, gateway errors and network failures are `Unreachable`, and other
/// refusals (bad credentials, bad endpoint) are `Rejected`.
pub async fn connect(
    endpoint: &WsEndpoint,
) -> Result<(ClientReceiver, ClientSender), ConnectError> {
    let request = endpoint.request()?;
    match tokio_tungstenite::connect_async_with_config(
        request,
        Some(config()),
        /*disable_nagle*/ true,
    )
    .await
    {
        Ok((stream, _)) => Ok(split(stream)),
        Err(error) => Err(ConnectError::new(classify(&error), error)),
    }
}

/// Maps a client handshake failure to its connection class.
fn classify(error: &WsError) -> ConnectFailure {
    match error {
        WsError::Http(response) => match response.status() {
            StatusCode::NOT_FOUND => ConnectFailure::NotFound,
            StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => ConnectFailure::Unreachable,
            _ => ConnectFailure::Rejected,
        },
        WsError::Url(_) | WsError::HttpFormat(_) | WsError::Tls(_) => ConnectFailure::Rejected,
        WsError::Io(_)
        | WsError::ConnectionClosed
        | WsError::AlreadyClosed
        | WsError::Capacity(_)
        | WsError::Protocol(_)
        | WsError::WriteBufferFull(_)
        | WsError::Utf8(_)
        | WsError::AttackAttempt => ConnectFailure::Unreachable,
    }
}

/// Accepts control connections on one TCP address and path. Requests for any other path are
/// refused during the upgrade, so a misrouted client never reaches session admission.
pub struct WsAcceptor {
    listener: TcpListener,
    path: Arc<str>,
}

impl WsAcceptor {
    /// Wraps a bound listener; `path` must be the exact request path, such as `/ora-node/v1`.
    pub fn new(listener: TcpListener, path: impl Into<Arc<str>>) -> Self {
        Self {
            listener,
            path: path.into(),
        }
    }

    /// Reports the bound address, including an ephemeral port chosen by the OS.
    pub fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }
}

/// Completes the server upgrade only for the configured path.
async fn upgrade(stream: TcpStream, path: &str) -> Result<WebSocketStream<TcpStream>, WsError> {
    // tungstenite's handshake callback signature fixes the refusal type; it cannot be boxed.
    #[allow(clippy::result_large_err)]
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        if request.uri().path() == path {
            return Ok(response);
        }
        let mut refusal = ErrorResponse::new(Some("unknown Node path".into()));
        *refusal.status_mut() = StatusCode::NOT_FOUND;
        Err(refusal)
    };
    tokio_tungstenite::accept_hdr_async_with_config(stream, check, Some(config())).await
}

impl Acceptor for WsAcceptor {
    type Pending = TcpStream;
    type Receiver = WsReceiver<TcpStream>;
    type Sender = WsSender<TcpStream>;

    /// Waits for the next TCP connection; the upgrade happens in `open` or `reject_busy`.
    async fn accept(&self) -> io::Result<TcpStream> {
        self.listener.accept().await.map(|(stream, _)| stream)
    }

    /// Performs the WebSocket upgrade under the caller's deadline.
    async fn open(
        &self,
        pending: TcpStream,
    ) -> Result<(Self::Receiver, Self::Sender), TransportError> {
        Ok(split(upgrade(pending, &self.path).await?))
    }

    /// Upgrades and then closes with the busy code: an HTTP refusal would reach the Controller as a
    /// router gateway error indistinguishable from an unreachable Node.
    fn reject_busy(&self, pending: TcpStream) -> impl Future<Output = ()> + Send + 'static {
        let path = self.path.clone();
        async move {
            let Ok(mut stream) = upgrade(pending, &path).await else {
                return;
            };
            let close = CloseFrame {
                code: CloseCode::from(CONTROL_SESSION_BUSY_CLOSE_CODE),
                reason: "control session busy".into(),
            };
            if stream.close(Some(close)).await.is_err() {
                return;
            }
            // Wait for the peer's close reply so the socket is not reset before our close frame
            // is read; the caller bounds this wait.
            while let Some(Ok(_)) = stream.next().await {}
        }
    }
}
