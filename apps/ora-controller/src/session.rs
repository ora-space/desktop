mod commands;
mod control;
mod downloads;
mod grants;
mod relay;
use super::*;
use ora_node_transport::{
    CloseReason, ConnectError, FrameReceiver, FrameSender, TransportError, close_connection, ipc,
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
    /// The peer violated the protocol, or closed the connection saying this side did.
    #[error("protocol failure: {0}")]
    Protocol(String),
    /// The peer is not the configured Node with the needed capability, or closed the connection
    /// because this Controller does not own it.
    #[error("identity mismatch: {0}")]
    Mismatch(String),
    /// The Node sent nothing, or stopped reading, within the I/O deadline.
    #[error("Node silent past the I/O deadline")]
    Silent,
    /// An established connection ended or failed.
    #[error("connection lost: {0}")]
    Disconnected(String),
    /// Persistence refused or could not confirm a step; nothing was acknowledged for it.
    #[error(transparent)]
    Store(#[from] Error),
}

impl SessionError {
    /// What to tell the Node when this failure ends an established session. Failures the Node
    /// caused by closing need no close of our own; attempting one would only fail fast.
    fn close_reason(&self) -> Option<CloseReason> {
        match self {
            Self::Protocol(_) => Some(CloseReason::ProtocolViolation),
            Self::Mismatch(_) => Some(CloseReason::IdentityMismatch),
            Self::Silent => Some(CloseReason::PeerSilent),
            Self::Store(_) => Some(CloseReason::InternalError),
            Self::Configuration | Self::Connect(_) | Self::Busy | Self::Disconnected(_) => None,
        }
    }
}

impl From<TransportError> for SessionError {
    /// Separates frame-level violations from ordinary connection loss, and keeps the Node's
    /// verdict when it closed for a protocol or identity reason.
    fn from(error: TransportError) -> Self {
        if error.is_busy() {
            return Self::Busy;
        }
        if error.is_closed_for(CloseReason::ProtocolViolation) {
            return Self::Protocol(error.to_string());
        }
        if error.is_closed_for(CloseReason::IdentityMismatch) {
            return Self::Mismatch(error.to_string());
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

/// What a session tells its owner beyond its own result. A static Node's handshake proves its
/// configured identity to the store; the Cloud adapter's sandbox sessions report the handshake to
/// Cloud and stop a Workspace's clone step when the Node cannot tell an execution's outcome. Callbacks run on the session task and must not
/// block it.
pub(crate) trait SessionObserver: Send + Sync {
    /// The handshake completed with this Node incarnation; the session is live from here on.
    fn established(&self, node: &NodeRuntimeIdentity);
    /// Records the negotiated capabilities before publishing the connected identity.
    fn capabilities(&self, _capabilities: &[NodeCapability]) {}
    /// The Node still answers `Unknown` for an execution after this connection retransmitted its
    /// original command once. It may be a retained uncertain attempt or a command the Node has not
    /// admitted yet; the owner decides how long that may last.
    fn unresolved(&self, execution: &ExecutionId);
    /// The Node reported the execution as accepted, running or completed, which ends any earlier
    /// unresolved report for it.
    fn answered(&self, execution: &ExecutionId);
}

/// The observer of sessions nobody watches.
pub(crate) struct Unobserved;

impl SessionObserver for Unobserved {
    fn established(&self, _node: &NodeRuntimeIdentity) {}

    fn unresolved(&self, _execution: &ExecutionId) {}

    fn answered(&self, _execution: &ExecutionId) {}
}

/// Bounds one I/O step; an elapsed deadline is connection loss, not an execution outcome.
async fn bounded<T>(
    deadline: Duration,
    step: impl std::future::Future<Output = Result<T, SessionError>>,
) -> Result<T, SessionError> {
    timeout(deadline, step)
        .await
        .map_err(|_| SessionError::Silent)?
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
    run_session_until(store, target, config, std::future::pending()).await
}

/// Like [`run_session`], but returns `Ok(())` once `stop` completes, after telling the Node the
/// Controller is shutting down so its logs show a deliberate end rather than a lost connection.
pub async fn run_session_until<S: CoordinationStore>(
    store: &S,
    target: &NodeTarget,
    config: &SessionConfig,
    stop: impl std::future::Future<Output = ()>,
) -> Result<(), SessionError> {
    run_observed_session(store, target, config, stop, &Unobserved).await
}

/// Like [`run_session_until`], reporting the handshake and unresolved executions to `observer`.
pub(crate) async fn run_observed_session<S: CoordinationStore, O: SessionObserver>(
    store: &S,
    target: &NodeTarget,
    config: &SessionConfig,
    stop: impl std::future::Future<Output = ()>,
    observer: &O,
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
            drive(
                store,
                &target.node_id,
                config,
                receiver,
                writer,
                stop,
                observer,
            )
            .await
        }
        NodeEndpoint::WebSocket(endpoint) => {
            let (receiver, writer) = timeout(deadline, websocket::connect(endpoint))
                .await
                .map_err(unreachable)??;
            drive(
                store,
                &target.node_id,
                config,
                receiver,
                writer,
                stop,
                observer,
            )
            .await
        }
    }
}

/// Runs one established session until it fails or `stop` completes, then closes the connection
/// with the matching reason under the I/O deadline.
async fn drive<S: CoordinationStore, R: FrameReceiver, W: FrameSender, O: SessionObserver>(
    store: &S,
    node_id: &NodeId,
    config: &SessionConfig,
    mut receiver: R,
    mut writer: W,
    stop: impl std::future::Future<Output = ()>,
    observer: &O,
) -> Result<(), SessionError> {
    let deadline = Duration::from_millis(config.io_timeout_ms);
    let (result, reason) = tokio::select! {
        result = coordinate(store, node_id, config, &mut receiver, &mut writer, observer) => {
            let Err(error) = result;
            let reason = error.close_reason();
            (Err(error), reason)
        }
        () = stop => (Ok(()), Some(CloseReason::Shutdown)),
    };
    if let Some(reason) = reason {
        let _ = timeout(
            deadline,
            close_connection(&mut receiver, &mut writer, reason),
        )
        .await;
    }
    result
}

/// Runs the handshake and the coordination loop over any frame transport; it only ends by failing.
async fn coordinate<S: CoordinationStore, R: FrameReceiver, W: FrameSender, O: SessionObserver>(
    store: &S,
    node_id: &NodeId,
    config: &SessionConfig,
    receiver: &mut R,
    writer: &mut W,
    observer: &O,
) -> Result<std::convert::Infallible, SessionError> {
    let deadline = Duration::from_millis(config.io_timeout_ms);
    let id = store.id().clone();
    let hello = ControllerToNodeMessage::Hello(HelloMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: Hello {
            controller_id: id.clone(),
            supported_versions: vec![CURRENT_PROTOCOL_VERSION],
        },
    });
    bounded(deadline, transmit(writer, &hello)).await?;
    let greeting = bounded(deadline, receive(receiver)).await?;
    let Some(NodeToControllerMessage::HelloAccepted(hello)) = greeting else {
        return Err(SessionError::Protocol("Node did not accept Hello".into()));
    };
    if hello.payload.node.node_id != *node_id
        || !hello
            .payload
            .capabilities
            .contains(&NodeCapability::RepositoryClone)
    {
        return Err(SessionError::Mismatch(
            "Node identity or capability mismatch".into(),
        ));
    }
    if store.requires_runtime_control()
        && !hello
            .payload
            .capabilities
            .contains(&NodeCapability::RuntimeControl)
    {
        return Err(SessionError::Mismatch(
            "Node lacks mandatory runtime_control capability".into(),
        ));
    }
    let agent_capable = hello
        .payload
        .capabilities
        .contains(&NodeCapability::AgentSession);
    let delivery_capable = hello
        .payload
        .capabilities
        .contains(&NodeCapability::RevisionDelivery);
    let restore_capable = hello
        .payload
        .capabilities
        .contains(&NodeCapability::RevisionRestore);
    observer.capabilities(&hello.payload.capabilities);
    let capabilities = hello.payload.capabilities;
    let identity = hello.payload.node;
    observer.established(&identity);
    let mut tick = interval(Duration::from_millis(config.query_interval_ms));
    // Slow authority reads must not accumulate catch-up queries ahead of terminal evidence.
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut relay = relay::Relay::new();
    let mut commands = commands::Commands::new(
        store.clone(),
        identity.clone(),
        Duration::from_millis(config.query_interval_ms),
        agent_capable,
    );
    let mut grants = grants::Grants::new(store.clone(), identity.clone(), delivery_capable);
    let mut downloads = downloads::Downloads::new(store.clone(), identity.clone(), restore_capable);
    let mut control = control::Control::new(
        store.clone(),
        identity.clone(),
        Duration::from_millis(config.query_interval_ms),
        capabilities,
        commands.input.clone(),
        grants.input.clone(),
    );
    let mut settled = std::collections::HashSet::new();
    loop {
        // One deadline spans the whole wait across query ticks; frame receipt itself is cancel-safe.
        let read = bounded(deadline, receive(receiver));
        tokio::pin!(read);
        let message = loop {
            tokio::select! {
                biased;
                message = &mut read => break message?.ok_or_else(|| SessionError::Disconnected("Node closed the connection".into()))?,
                result = relay.tasks.join_next(), if !relay.tasks.is_empty() => return Err(worker_failure(result)),
                result = commands.tasks.join_next(), if !commands.tasks.is_empty() => return Err(worker_failure(result)),
                Some(reply) = relay.outgoing.recv() => {
                    let ack = match reply {
                        relay::Receipt::Thread(ack) => ack,
                        relay::Receipt::Terminal(ack) => { settled.insert(ack.execution_id.clone()); ack },
                    };
                    observer.answered(&ack.execution_id);
                    bounded(deadline, transmit(writer, &ControllerToNodeMessage::EventAck(ack))).await?;
                }
                Some(reply) = commands.outgoing.recv(), if agent_capable => {
                    bounded(deadline, transmit(writer, &reply)).await?;
                }
                result = grants.tasks.join_next(), if !grants.tasks.is_empty() => return Err(worker_failure(result)),
                // Grants go to the wire and nowhere else; the frame is never logged.
                Some(grant) = grants.outgoing.recv(), if delivery_capable => {
                    bounded(deadline, transmit(writer, &grant)).await?;
                }
                result = downloads.tasks.join_next(), if !downloads.tasks.is_empty() => return Err(worker_failure(result)),
                // Like upload grants, download answers go to the wire and nowhere else.
                Some(answer) = downloads.outgoing.recv(), if restore_capable => {
                    bounded(deadline, transmit(writer, &answer)).await?;
                }
                result = control.tasks.join_next(), if !control.tasks.is_empty() => return Err(worker_failure(result)),
                Some(action) = control.outgoing.recv() => match action {
                    control::Action::Send(reply) => bounded(deadline, transmit(writer, &reply)).await?,
                    control::Action::Answered(execution) => observer.answered(&execution),
                    control::Action::Unresolved(execution) => { if !settled.contains(&execution) { observer.unresolved(&execution); } },
                },
                _ = tick.tick() => {
                    let heartbeat = ControllerToNodeMessage::Heartbeat(ControllerHeartbeatMessage { protocol_version: CURRENT_PROTOCOL_VERSION, payload: ControllerHeartbeat { controller_id: id.clone() } });
                    bounded(deadline, transmit(writer, &heartbeat)).await?;
                }
            }
        };
        message.validate().map_err(Error::from)?;
        match &message {
            NodeToControllerMessage::ThreadEvent(event) if agent_capable => {
                relay.push(store, &identity, relay::Event::Thread(event.clone()))?;
                continue;
            }
            NodeToControllerMessage::AgentSessionEnded(event) if agent_capable => {
                relay.push(store, &identity, relay::Event::End(event.clone()))?;
                continue;
            }
            NodeToControllerMessage::SessionCommandAccepted(reply) if agent_capable => {
                commands
                    .input
                    .try_send(commands::Reply::Command {
                        operation: reply.operation_id.clone(),
                        execution: reply.execution_id.clone(),
                        command: reply.payload.command_id.clone(),
                    })
                    .map_err(|_| SessionError::Protocol("command reply queue overflow".into()))?;
                continue;
            }
            NodeToControllerMessage::SessionCommandRejected(reply) if agent_capable => {
                commands
                    .input
                    .try_send(commands::Reply::Command {
                        operation: reply.operation_id.clone(),
                        execution: reply.execution_id.clone(),
                        command: reply.payload.command_id.clone(),
                    })
                    .map_err(|_| SessionError::Protocol("command reply queue overflow".into()))?;
                continue;
            }
            NodeToControllerMessage::UploadGrantNeeded(need) if delivery_capable => {
                if need.payload.node_id != identity.node_id {
                    return Err(SessionError::Protocol(
                        "grant request names another Node".into(),
                    ));
                }
                grants
                    .input
                    .try_send(grants::Request::Needed {
                        operation: need.operation_id.clone(),
                        execution: need.execution_id.clone(),
                        checksums: need.payload.checksums.clone(),
                    })
                    .map_err(|_| SessionError::Protocol("grant request queue overflow".into()))?;
                continue;
            }
            NodeToControllerMessage::DownloadGrantNeeded(need) if restore_capable => {
                if need.payload.node_id != identity.node_id {
                    return Err(SessionError::Protocol(
                        "download grant request names another Node".into(),
                    ));
                }
                downloads
                    .input
                    .try_send(downloads::Request {
                        operation: need.operation_id.clone(),
                        execution: need.execution_id.clone(),
                    })
                    .map_err(|_| {
                        SessionError::Protocol("download grant request queue overflow".into())
                    })?;
                continue;
            }
            NodeToControllerMessage::Heartbeat(heartbeat) if heartbeat.payload.node == identity => {
                continue;
            }
            _ => {}
        }
        control
            .input
            .try_send(message)
            .map_err(|_| SessionError::Protocol("control reply queue overflow".into()))?;
    }
}

/// Worker failures end this connection without inventing an acknowledgement; reconnect replays.
fn worker_failure(
    result: Option<Result<Result<(), Error>, tokio::task::JoinError>>,
) -> SessionError {
    match result {
        Some(Ok(Err(error))) => SessionError::Store(error),
        Some(Err(error)) => SessionError::Protocol(format!("session worker failed: {error}")),
        _ => SessionError::Disconnected("session worker ended".into()),
    }
}
