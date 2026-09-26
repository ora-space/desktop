use super::*;
use ora_node_transport::{
    Acceptor, FrameReceiver, FrameSender, ipc::IpcAcceptor, websocket::WsAcceptor,
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc as async_queue},
    time::{Instant, interval, timeout, timeout_at},
};

/// Controller messages one session may leave unanswered. One admission slot stays free for the
/// replay pass, so a full pipeline never makes the worker queue itself refuse a request.
const UNANSWERED_REQUEST_LIMIT: usize = ADMISSION_QUEUE_BOUND - 1;

/// The worker's eventual answer to one queued request.
type Response = oneshot::Receiver<Result<Vec<NodeToControllerMessage>, String>>;

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
        let (receiver, writer) = timeout(deadline, acceptor.open(pending))
            .await
            .map_err(io::Error::other)?
            .map_err(io::Error::other)?;
        connected(receiver, writer, config, info, sender, active.clone()).await
    };
    tokio::pin!(session);
    let mut tick = interval(Duration::from_millis(/*millis*/ 25));
    let result = loop {
        tokio::select! {
            result = &mut session => break result,
            accepted = acceptor.accept() => {
                // A WebSocket refusal needs its own upgrade; running it beside the session keeps
                // heartbeats and command handling independent of the rejected peer.
                tokio::spawn(timeout(deadline, acceptor.reject_busy(accepted?)));
            }
            _ = tick.tick() => { if shutdown.requested() { break Ok(()); } }
        }
    };
    *active
        .lock()
        .map_err(|_| io::Error::other("session admission poisoned"))? = false;
    if result.is_err() {
        ora_logging::ora_warn!(
            "Node control session closed; durable execution responsibility retained"
        );
    }
    Ok(())
}

/// Reads and validates the next Controller message; `None` is a clean end of the connection.
async fn receive<R: FrameReceiver>(
    receiver: &mut R,
) -> io::Result<Option<ControllerToNodeMessage>> {
    match receiver.recv().await.map_err(io::Error::other)? {
        Some(frame) => decode_controller_frame(&frame)
            .map(Some)
            .map_err(io::Error::other),
        None => Ok(None),
    }
}

/// Encodes and sends one Node message as a single frame.
async fn transmit<W: FrameSender>(
    writer: &mut W,
    message: &NodeToControllerMessage,
) -> io::Result<()> {
    let frame = encode_node_frame(message).map_err(io::Error::other)?;
    writer.send(frame).await.map_err(io::Error::other)
}

/// Queues bounded requests with a revocable admission identity without waiting for the worker;
/// closing a connection does not cancel accepted work.
fn enqueue(
    sender: &mpsc::SyncSender<Work>,
    active: &Arc<Mutex<bool>>,
    request: Request,
) -> io::Result<Response> {
    let (reply, response) = oneshot::channel();
    sender
        .try_send(Work {
            active: active.clone(),
            request,
            reply,
        })
        .map_err(|_| io::Error::other("Node admission queue unavailable"))?;
    Ok(response)
}

/// Waits for the worker's answer; a dropped reply means the worker has stopped.
async fn settle(response: Response) -> io::Result<Vec<NodeToControllerMessage>> {
    response
        .await
        .map_err(io::Error::other)?
        .map_err(io::Error::other)
}

/// Runs reading, answering, replay and writing as independent futures so neither Git in the worker
/// nor a slow peer stops the others; the first to fail ends the whole session.
async fn connected<R: FrameReceiver, W: FrameSender>(
    mut receiver: R,
    mut writer: W,
    config: &ControlConfig,
    info: &SessionInfo,
    sender: &mpsc::SyncSender<Work>,
    active: Arc<Mutex<bool>>,
) -> io::Result<()> {
    let deadline = Duration::from_millis(config.frame_timeout_ms);
    let greeting = timeout(deadline, receive(&mut receiver))
        .await
        .map_err(io::Error::other)??;
    let Some(ControllerToNodeMessage::Hello(hello)) = greeting else {
        return Err(io::Error::other("expected Hello"));
    };
    if hello.payload.controller_id != info.controller {
        return Err(io::Error::other("Controller does not own this Node"));
    }
    let response = NodeToControllerMessage::HelloAccepted(HelloAcceptedMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        payload: HelloAccepted {
            selected_version: CURRENT_PROTOCOL_VERSION,
            node: info.identity.clone(),
            capabilities: info.capabilities.clone(),
        },
    });
    timeout(deadline, transmit(&mut writer, &response))
        .await
        .map_err(io::Error::other)??;
    let (outgoing, mut messages) = async_queue::channel(/*buffer*/ 16);
    // Permits travel with each request until it is answered, so the bound covers the request the
    // answering future is waiting on as well as the ones queued behind it.
    let unanswered = Arc::new(Semaphore::new(UNANSWERED_REQUEST_LIMIT));
    let (answer_later, mut answering) = async_queue::unbounded_channel();
    // Reading never waits for the worker: a peer's close, a router's ping and a truncated frame
    // must be observed while Git occupies the worker, or the control slot stays held for a peer
    // that is already gone and its reconnection is refused as busy.
    let read = async {
        loop {
            let message = timeout(deadline, receive(&mut receiver))
                .await
                .map_err(io::Error::other)??;
            let Some(message) = message else {
                return Ok::<(), io::Error>(());
            };
            // The per-frame deadline above is the Controller liveness deadline; its idle heartbeat
            // only renews it and must not queue behind Git in the worker.
            match &message {
                ControllerToNodeMessage::Heartbeat(heartbeat)
                    if heartbeat.payload.controller_id != info.controller =>
                {
                    return Err(io::Error::other(
                        "heartbeat from a Controller that does not own this Node",
                    ));
                }
                ControllerToNodeMessage::Heartbeat(_) => continue,
                _ => {}
            }
            let Ok(permit) = unanswered.clone().try_acquire_owned() else {
                // The Controller polls status on a timer without waiting for replies, so a long
                // Git pass fills the pipeline with queries. Shedding the excess is safe because
                // a query changes nothing and the Controller asks again; a command must not be
                // lost silently, and closing makes the Controller reconcile it by query.
                if matches!(message, ControllerToNodeMessage::GetExecutionStatus(_)) {
                    continue;
                }
                return Err(io::Error::other(
                    "Controller exceeded unanswered request limit",
                ));
            };
            // Enqueueing in read order keeps the worker's FIFO equal to the Controller's order.
            let response = enqueue(sender, &active, Request::Message(message))?;
            answer_later
                .send((Instant::now() + deadline, response, permit))
                .map_err(io::Error::other)?;
        }
    };
    // Git may occupy the worker. Each admission wait is bounded from the moment its frame arrived,
    // independently of heartbeats, so revocation invalidates queued work; already durable
    // executions are not canceled. Answers leave in request order because the worker is FIFO.
    let answer = async {
        while let Some((expiry, response, _permit)) = answering.recv().await {
            let replies = timeout_at(expiry, settle(response))
                .await
                .map_err(io::Error::other)??;
            for reply in replies {
                outgoing.send(reply).await.map_err(io::Error::other)?;
            }
        }
        Ok::<(), io::Error>(())
    };
    let replay = async {
        let mut tick = interval(Duration::from_millis(config.heartbeat_ms));
        loop {
            tick.tick().await;
            for event in settle(enqueue(sender, &active, Request::Replay)?).await? {
                outgoing.send(event).await.map_err(io::Error::other)?;
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), io::Error>(())
    };
    let write = async {
        let mut tick = interval(Duration::from_millis(config.heartbeat_ms));
        loop {
            let message = tokio::select! {
                message = messages.recv() => { let Some(message) = message else { return Ok::<(), io::Error>(()); }; message }
                _ = tick.tick() => NodeToControllerMessage::Heartbeat(HeartbeatMessage { protocol_version: CURRENT_PROTOCOL_VERSION, payload: Heartbeat { node: info.identity.clone() } }),
            };
            timeout(deadline, transmit(&mut writer, &message))
                .await
                .map_err(io::Error::other)??;
        }
    };
    tokio::select! {
        result = read => result,
        result = answer => result,
        result = write => result,
        result = replay => result,
    }
}
