use super::*;
use ora_node_transport::{
    Acceptor, CloseReason, FrameReceiver, FrameSender, TransportError, close_connection,
    ipc::IpcAcceptor, websocket::WsAcceptor,
};
use tokio::{
    net::TcpListener,
    sync::mpsc as async_queue,
    time::{interval, timeout},
};

/// Binds the single configured entry and serves it; the match only picks the monomorphized
/// acceptor, so admission and session handling are the same code for every transport.
pub(super) async fn serve(
    config: ControlConfig,
    info: SessionInfo,
    sender: mpsc::SyncSender<Work>,
    shutdown: Shutdown,
) -> io::Result<()> {
    match &config.listen {
        ControlListen::Ipc { path } => {
            // SAFETY: this reads identity only; the worker already holds the stable Node database lease.
            let uid = unsafe { libc::geteuid() };
            let listener = ora_utils::local_ipc::bind_private_endpoint(
                path,
                uid,
                Duration::from_millis(config.frame_timeout_ms),
            )
            .await?;
            ora_logging::ora_info!("Node IPC listening");
            listen(
                IpcAcceptor::new(listener),
                &config,
                &info,
                &sender,
                &shutdown,
            )
            .await
        }
        ControlListen::WebSocket { bind, path } => {
            let listener = TcpListener::bind(bind).await?;
            let acceptor = WsAcceptor::new(listener, path.as_str());
            ora_logging::ora_info!(address = %acceptor.local_addr()?, path = %path, "Node WebSocket listening");
            listen(acceptor, &config, &info, &sender, &shutdown).await
        }
    }
}

/// Rejects concurrent connections while a session owns admission, including its handshake window.
async fn listen<A: Acceptor>(
    acceptor: A,
    config: &ControlConfig,
    info: &SessionInfo,
    sender: &mpsc::SyncSender<Work>,
    shutdown: &Shutdown,
) -> io::Result<()> {
    let mut tick = interval(Duration::from_millis(/*millis*/ 25));
    while !shutdown.requested() {
        tokio::select! {
            accepted = acceptor.accept() => {
                run_exclusive(&acceptor, accepted?, config, info, sender, shutdown).await?;
            }
            _ = tick.tick() => {}
        }
    }
    Ok(())
}

/// The rejection loop stays live during long Git operations and never replaces the existing session.
async fn run_exclusive<A: Acceptor>(
    acceptor: &A,
    pending: A::Pending,
    config: &ControlConfig,
    info: &SessionInfo,
    sender: &mpsc::SyncSender<Work>,
    shutdown: &Shutdown,
) -> io::Result<()> {
    let active = Arc::new(Mutex::new(true));
    let deadline = Duration::from_millis(config.frame_timeout_ms);
    let session = async {
        // The transport handshake counts as part of the admission window it already owns.
        let (mut receiver, mut writer) = tokio::select! {
            opened = timeout(deadline, acceptor.open(pending)) => opened
                .map_err(io::Error::other)?
                .map_err(io::Error::other)?,
            () = stopped(shutdown) => return Ok(()),
        };
        // Shutdown is observed here rather than by dropping the session, so the Controller learns
        // the Node is stopping instead of seeing a lost connection.
        let (result, reason) = tokio::select! {
            result = connected(&mut receiver, &mut writer, config, info, sender, active.clone()) => match result {
                Ok(()) => (Ok(()), None),
                Err(failure) => (Err(failure.error), failure.close),
            },
            () = stopped(shutdown) => (Ok(()), Some(CloseReason::Shutdown)),
        };
        // A stopping worker can fail the session before the shutdown poll notices the request;
        // the Controller should still learn that the Node is stopping.
        let reason = reason.map(|reason| {
            if shutdown.requested() {
                CloseReason::Shutdown
            } else {
                reason
            }
        });
        match reason {
            // The runtime ends soon after shutdown, so this close cannot be left to a task.
            Some(CloseReason::Shutdown) => {
                let _ = timeout(
                    deadline,
                    close_connection(&mut receiver, &mut writer, CloseReason::Shutdown),
                )
                .await;
            }
            // Admission is released without waiting for the Controller to finish closing, so a
            // peer that lingers after the close cannot turn the next connection away as busy.
            Some(reason) => {
                tokio::spawn(timeout(deadline, async move {
                    close_connection(&mut receiver, &mut writer, reason).await;
                }));
            }
            None => {}
        }
        result
    };
    tokio::pin!(session);
    let result = loop {
        tokio::select! {
            result = &mut session => break result,
            accepted = acceptor.accept() => {
                // A WebSocket refusal needs its own upgrade; running it beside the session keeps
                // heartbeats and command handling independent of the rejected peer.
                tokio::spawn(timeout(deadline, acceptor.reject_busy(accepted?)));
            }
        }
    };
    *active
        .lock()
        .map_err(|_| io::Error::other("session admission poisoned"))? = false;
    if let Err(error) = result {
        ora_logging::ora_warn!(
            error = %error,
            "Node control session closed; durable execution responsibility retained"
        );
    }
    Ok(())
}

/// Completes once shutdown is requested; the flag is polled because it is shared with blocking code.
async fn stopped(shutdown: &Shutdown) {
    let mut tick = interval(Duration::from_millis(/*millis*/ 25));
    while !shutdown.requested() {
        tick.tick().await;
    }
}

/// A failed session and what the Node tells the Controller before closing; `None` when the
/// Controller's side already ended the connection.
struct Failure {
    close: Option<CloseReason>,
    error: io::Error,
}

impl Failure {
    /// A failure this Node detected and reports with `reason`.
    fn local(
        reason: CloseReason,
        error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self {
            close: Some(reason),
            error: io::Error::other(error),
        }
    }

    /// The Controller missed the frame deadline, whether by not sending or by not reading.
    fn silent() -> Self {
        Self::local(CloseReason::PeerSilent, "frame deadline elapsed")
    }
}

impl From<TransportError> for Failure {
    /// Malformed input is the Controller's protocol violation; anything else means the
    /// connection is already gone.
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Frame(_) | TransportError::UnexpectedMessage => {
                Self::local(CloseReason::ProtocolViolation, error)
            }
            TransportError::Io(_)
            | TransportError::Closed { .. }
            | TransportError::WebSocket(_) => Self {
                close: None,
                error: io::Error::other(error),
            },
        }
    }
}

/// Reads and validates the next Controller message; `None` is a clean end of the connection.
async fn receive<R: FrameReceiver>(
    receiver: &mut R,
) -> Result<Option<ControllerToNodeMessage>, Failure> {
    match receiver.recv().await? {
        Some(frame) => decode_controller_frame(&frame)
            .map(Some)
            .map_err(|error| Failure::local(CloseReason::ProtocolViolation, error)),
        None => Ok(None),
    }
}

/// Encodes and sends one Node message as a single frame.
async fn transmit<W: FrameSender>(
    writer: &mut W,
    message: &NodeToControllerMessage,
) -> Result<(), Failure> {
    let frame = encode_node_frame(message)
        .map_err(|error| Failure::local(CloseReason::InternalError, error))?;
    Ok(writer.send(frame).await?)
}

/// Queues bounded requests with a revocable admission identity; closing a connection does not cancel accepted work.
async fn request(
    sender: &mpsc::SyncSender<Work>,
    active: &Arc<Mutex<bool>>,
    request: Request,
) -> Result<Vec<NodeToControllerMessage>, Failure> {
    let (reply, response) = oneshot::channel();
    sender
        .try_send(Work {
            active: active.clone(),
            request,
            reply,
        })
        .map_err(|_| {
            Failure::local(
                CloseReason::InternalError,
                "Node admission queue unavailable",
            )
        })?;
    response
        .await
        .map_err(|error| Failure::local(CloseReason::InternalError, error))?
        .map_err(|rejection| Failure::local(rejection.close, rejection.message))
}

/// Keeps one reader future alive per frame and gives heartbeat/output independent execution from Git.
async fn connected<R: FrameReceiver, W: FrameSender>(
    receiver: &mut R,
    writer: &mut W,
    config: &ControlConfig,
    info: &SessionInfo,
    sender: &mpsc::SyncSender<Work>,
    active: Arc<Mutex<bool>>,
) -> Result<(), Failure> {
    let deadline = Duration::from_millis(config.frame_timeout_ms);
    let greeting = timeout(deadline, receive(receiver))
        .await
        .map_err(|_| Failure::silent())??;
    let Some(ControllerToNodeMessage::Hello(hello)) = greeting else {
        return Err(Failure::local(
            CloseReason::ProtocolViolation,
            "expected Hello",
        ));
    };
    if hello.payload.controller_id != info.controller {
        return Err(Failure::local(
            CloseReason::IdentityMismatch,
            "Controller does not own this Node",
        ));
    }
    let response = NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: HelloAccepted {
            selected_version: CURRENT_PROTOCOL_VERSION,
            node: info.identity.clone(),
            capabilities: info.capabilities.clone(),
        },
    });
    timeout(deadline, transmit(writer, &response))
        .await
        .map_err(|_| Failure::silent())??;
    let (outgoing, mut messages) = async_queue::channel(/*buffer*/ 16);
    // The write task owns the receiving end of `outgoing`, so a failed send means the session is
    // already ending.
    let queue_closed = |_| Failure::local(CloseReason::InternalError, "session output closed");
    let read = async {
        loop {
            let message = timeout(deadline, receive(receiver))
                .await
                .map_err(|_| Failure::silent())??;
            let Some(message) = message else {
                return Ok::<(), Failure>(());
            };
            // The per-frame deadline above is the Controller liveness deadline; its idle heartbeat
            // only renews it and must not queue behind Git in the worker.
            match &message {
                ControllerToNodeMessage::Heartbeat(heartbeat)
                    if heartbeat.payload.controller_id != info.controller =>
                {
                    return Err(Failure::local(
                        CloseReason::IdentityMismatch,
                        "heartbeat from a Controller that does not own this Node",
                    ));
                }
                ControllerToNodeMessage::Heartbeat(_) => continue,
                _ => {}
            }
            // Git may occupy the worker. Bound admission waiting independently of heartbeats so
            // revocation invalidates queued work; already durable executions are not canceled.
            let replies = timeout(
                deadline,
                request(sender, &active, Request::Message(message)),
            )
            .await
            .map_err(|_| {
                Failure::local(CloseReason::InternalError, "admission deadline elapsed")
            })??;
            for reply in replies {
                outgoing.send(reply).await.map_err(queue_closed)?;
            }
        }
    };
    let replay = async {
        let mut tick = interval(Duration::from_millis(config.heartbeat_ms));
        loop {
            tick.tick().await;
            for event in request(sender, &active, Request::Replay).await? {
                outgoing.send(event).await.map_err(queue_closed)?;
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), Failure>(())
    };
    let write = async {
        let mut tick = interval(Duration::from_millis(config.heartbeat_ms));
        loop {
            let message = tokio::select! {
                message = messages.recv() => { let Some(message) = message else { return Ok::<(), Failure>(()); }; message }
                _ = tick.tick() => NodeToControllerMessage::Heartbeat(HeartbeatMessage { protocol_version: CURRENT_PROTOCOL_VERSION, payload: Heartbeat { node: info.identity.clone() } }),
            };
            timeout(deadline, transmit(writer, &message))
                .await
                .map_err(|_| Failure::silent())??;
        }
    };
    tokio::select! { result = read => result, result = write => result, result = replay => result }
}
