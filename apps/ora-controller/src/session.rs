use super::*;
use ora_node_transport::{
    ConnectError, FrameReceiver, FrameSender, TransportError, ipc,
    websocket::{self, WsEndpoint},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::{interval, timeout};

/// Deployment maps one persistent Node identity to how it is reached, never a checkout path.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeTarget {
    pub node_id: NodeId,
    pub endpoint: NodeEndpoint,
}

/// How the Controller reaches one Node. Vendor addressing and platform credentials are already
/// folded into a [`WsEndpoint`] by whoever provides it; sessions never see a vendor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum NodeEndpoint {
    /// A private local socket, protected by filesystem permissions.
    #[serde(rename = "ipc")]
    Ipc { path: PathBuf },
    /// A Node behind a platform WebSocket router, which alone authenticates the Controller.
    #[serde(rename = "websocket")]
    WebSocket(WsEndpoint),
}

/// Finite I/O deadlines and periodic queries keep sessions live independently of clone duration.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    pub io_timeout_ms: u64,
    pub query_interval_ms: u64,
}

/// Why a session ended. Every variant only means the connection is unavailable: the caller
/// reconnects with the same identities and no execution is failed, re-created or abandoned.
/// The classes exist so operators (and later sandbox lifecycle code) can tell them apart.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("invalid session configuration")]
    Configuration,
    /// No connection was established; the inner class separates unreachable, unknown and refused.
    #[error(transparent)]
    Connect(#[from] ConnectError),
    /// The Node already serves another live control session.
    #[error("Node refused the connection: another control session is live")]
    Busy,
    /// The peer violated the protocol or is not the configured Node with the needed capability.
    #[error("protocol failure: {0}")]
    Protocol(String),
    /// An established connection ended, failed or missed its I/O deadline.
    #[error("connection lost: {0}")]
    Disconnected(String),
    /// Persistence refused or could not confirm a step; nothing was acknowledged for it.
    #[error(transparent)]
    Store(#[from] Error),
}

impl From<TransportError> for SessionError {
    /// Separates frame-level violations from ordinary connection loss.
    fn from(error: TransportError) -> Self {
        if error.is_busy() {
            return Self::Busy;
        }
        match error {
            TransportError::Frame(_) | TransportError::UnexpectedMessage => {
                Self::Protocol(error.to_string())
            }
            TransportError::Io(_)
            | TransportError::Closed { .. }
            | TransportError::WebSocket(_) => Self::Disconnected(error.to_string()),
        }
    }
}

/// Bounds one I/O step; an elapsed deadline is connection loss, not an execution outcome.
async fn bounded<T>(
    deadline: Duration,
    step: impl std::future::Future<Output = Result<T, SessionError>>,
) -> Result<T, SessionError> {
    timeout(deadline, step)
        .await
        .map_err(|_| SessionError::Disconnected("I/O deadline elapsed".into()))?
}

/// Reads and validates the next Node message; `None` is a clean end of the connection.
async fn receive<R: FrameReceiver>(
    receiver: &mut R,
) -> Result<Option<NodeToControllerMessage>, SessionError> {
    match receiver.recv().await? {
        Some(frame) => decode_node_frame(&frame)
            .map(Some)
            .map_err(|error| SessionError::Protocol(error.to_string())),
        None => Ok(None),
    }
}

/// Encodes and sends one Controller message as a single frame.
async fn transmit<W: FrameSender>(
    writer: &mut W,
    message: &ControllerToNodeMessage,
) -> Result<(), SessionError> {
    let frame = encode_controller_frame(message)
        .map_err(|error| SessionError::Protocol(error.to_string()))?;
    Ok(writer.send(frame).await?)
}

/// Coordinates one connection; the caller reconnects using the same durable Controller, never new IDs.
/// Deployment overlap between store and Node state roots is validated when the runtime opens.
pub async fn run_session<S: CoordinationStore>(
    store: &S,
    target: &NodeTarget,
    config: &SessionConfig,
) -> Result<(), SessionError> {
    if config.io_timeout_ms == 0 || config.query_interval_ms == 0 {
        return Err(SessionError::Configuration);
    }
    let deadline = Duration::from_millis(config.io_timeout_ms);
    let unreachable = |_| {
        ConnectError::new(
            ora_node_transport::ConnectFailure::Unreachable,
            "connection deadline elapsed",
        )
    };
    // The endpoint variant only selects the monomorphized session; its behavior is shared.
    match &target.endpoint {
        NodeEndpoint::Ipc { path } => {
            if !path.is_absolute() {
                return Err(SessionError::Configuration);
            }
            let (receiver, writer) = timeout(deadline, ipc::connect(path))
                .await
                .map_err(unreachable)??;
            drive(store, &target.node_id, config, receiver, writer).await
        }
        NodeEndpoint::WebSocket(endpoint) => {
            let (receiver, writer) = timeout(deadline, websocket::connect(endpoint))
                .await
                .map_err(unreachable)??;
            drive(store, &target.node_id, config, receiver, writer).await
        }
    }
}

/// Runs the handshake and the coordination loop over any frame transport.
async fn drive<S: CoordinationStore, R: FrameReceiver, W: FrameSender>(
    store: &S,
    node_id: &NodeId,
    config: &SessionConfig,
    mut receiver: R,
    mut writer: W,
) -> Result<(), SessionError> {
    let deadline = Duration::from_millis(config.io_timeout_ms);
    let id = store.id().clone();
    let hello = ControllerToNodeMessage::Hello(HelloMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Hello {
            controller_id: id.clone(),
            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
        },
    });
    bounded(deadline, transmit(&mut writer, &hello)).await?;
    let greeting = bounded(deadline, receive(&mut receiver)).await?;
    let Some(NodeToControllerMessage::HelloAccepted(hello)) = greeting else {
        return Err(SessionError::Protocol("Node did not accept Hello".into()));
    };
    if hello.payload.node.node_id != *node_id
        || !hello
            .payload
            .capabilities
            .contains(&NodeCapability::RepositoryClone)
    {
        return Err(SessionError::Protocol(
            "Node identity or capability mismatch".into(),
        ));
    }
    let identity = hello.payload.node;
    let mut tick = interval(Duration::from_millis(config.query_interval_ms));
    let mut cursor = 0usize;
    // Unknown can also be a retained uncertain attempt. Repeated Unknown replies must not form
    // an immediate query/command feedback loop; one exact retransmission per connection is enough.
    let mut retransmitted = std::collections::HashSet::new();
    loop {
        // One deadline spans the whole wait across query ticks; frame receipt itself is cancel-safe.
        let read = bounded(deadline, receive(&mut receiver));
        tokio::pin!(read);
        let message = loop {
            tokio::select! {
                message = &mut read => break message?.ok_or_else(|| SessionError::Disconnected("Node closed the connection".into()))?,
                _ = tick.tick() => {
                    let command = {
                        let commands = store.pending_dispatches(node_id).await?;
                        if commands.is_empty() { None } else { let command = commands[cursor % commands.len()].clone(); cursor = cursor.wrapping_add(1); Some(command) }
                    };
                    // Every tick sends exactly one uplink frame: the Node treats its per-frame read
                    // deadline as Controller liveness, so an idle session must still send something.
                    let uplink = match command {
                        Some(command) => ControllerToNodeMessage::GetExecutionStatus(GetExecutionStatusMessage { protocol_version: CURRENT_PROTOCOL_VERSION, operation_id: command.operation_id, execution_id: command.execution_id, payload: GetExecutionStatus { node_id: node_id.clone() } }),
                        None => ControllerToNodeMessage::Heartbeat(ControllerHeartbeatMessage { protocol_version: CURRENT_PROTOCOL_VERSION, payload: ControllerHeartbeat { controller_id: id.clone() } }),
                    };
                    bounded(deadline, transmit(&mut writer, &uplink)).await?;
                }
            }
        };
        let ack = take_over(store, &identity, &message).await?;
        let reply = if let Some(ack) = ack {
            Some(ControllerToNodeMessage::EventAck(ack))
        } else if let NodeToControllerMessage::ExecutionStatus(status) = &message
            && status.payload.state == ExecutionState::Unknown
            && store.result(&status.execution_id).await?.is_none()
            && retransmitted.insert(status.execution_id.clone())
        {
            Some(ControllerToNodeMessage::CloneRepository(
                store
                    .original_dispatch(&identity, &status.operation_id, &status.execution_id)
                    .await?,
            ))
        } else {
            None
        };
        if let Some(reply) = reply {
            bounded(deadline, transmit(&mut writer, &reply)).await?;
        }
    }
}
