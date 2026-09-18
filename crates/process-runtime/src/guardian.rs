//! Independent trusted-local guardian with durable Run acceptance and rootless cleanup.

mod journal;
mod management;
mod runs;

use std::fs::File;
use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use management::Management;
use ora_process_protocol::{
    GUARDIAN_MAX_FRAME, GUARDIAN_WIRE_VERSION, GuardianAccess, GuardianBootstrap, GuardianChannel,
    GuardianReady, GuardianRequest, decode_guardian_payload, encode_guardian_frame,
};
use ora_utils::fs::LinuxFileLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use crate::ProcessStateError;

/// Owns an inherited locked description before any journal creation or endpoint publication.
///
/// Bootstrap EOF after delivery is not a liveness lease. Accepted KeepRunning workloads outlive
/// host connections; Control owns mutation and polling, while Io exposes bounded volatile output.
pub async fn serve_guardian_bootstrap(
    inherited_lock: File,
    mut bootstrap: UnixStream,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ProcessStateError> {
    // SAFETY: geteuid only queries the effective process identity.
    let owner = unsafe { libc::geteuid() };
    if bootstrap.peer_cred()?.uid() != owner {
        return Err(ProcessStateError::Rejected(
            "bootstrap peer identity mismatch",
        ));
    }
    let lock = LinuxFileLock::adopt_inherited(inherited_lock)?;
    let message: GuardianBootstrap = tokio::time::timeout(
        Duration::from_secs(/*secs*/ 5),
        read_message(&mut bootstrap),
    )
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "bootstrap timed out"))??;
    if message.version != GUARDIAN_WIRE_VERSION {
        return Err(ProcessStateError::Rejected(
            "unsupported guardian bootstrap version",
        ));
    }
    let access = message.access;
    drop(bootstrap);
    let management = Arc::new(Mutex::new(Management::new(
        journal::initialize(&access, &lock, owner)?,
        access.intent.clone(),
        lock,
    )?));
    let control = UnixListener::bind(
        access
            .scope_dir
            .join(GuardianChannel::Control.socket_name()),
    )?;
    let events = UnixListener::bind(access.scope_dir.join(GuardianChannel::Events.socket_name()))?;
    let io_listener = UnixListener::bind(access.scope_dir.join(GuardianChannel::Io.socket_name()))?;
    for channel in [
        GuardianChannel::Control,
        GuardianChannel::Events,
        GuardianChannel::Io,
    ] {
        std::fs::set_permissions(
            access.scope_dir.join(channel.socket_name()),
            std::fs::Permissions::from_mode(/*mode*/ 0o600),
        )?;
    }
    File::open(&access.scope_dir)?.sync_all()?;
    let mut control_workers = JoinSet::new();
    let mut event_workers = JoinSet::new();
    let mut io_workers = JoinSet::new();
    let mut reconcile = tokio::time::interval(Duration::from_millis(/*millis*/ 50));
    reconcile.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tokio::pin!(shutdown);
    let result = loop {
        // Fair selection lets stop/takeover traffic proceed even when reconciliation overruns a tick.
        tokio::select! {
            () = &mut shutdown => break Ok(()),
            _ = reconcile.tick() => management.lock().await.reconcile(),
            result = control_workers.join_next(), if !control_workers.is_empty() => {
                if let Some(Err(error)) = result { break Err(io::Error::other(error).into()); }
            }
            result = event_workers.join_next(), if !event_workers.is_empty() => {
                if let Some(Err(error)) = result { break Err(io::Error::other(error).into()); }
            }
            result = io_workers.join_next(), if !io_workers.is_empty() => {
                if let Some(Err(error)) = result { break Err(io::Error::other(error).into()); }
            }
            result = control.accept(), if control_workers.len() < 16 => {
                match result {
                    Ok((stream, _)) => { control_workers.spawn(serve_probe(stream, access.clone(), GuardianChannel::Control, owner, management.clone())); }
                    Err(error) => break Err(error.into()),
                }
            }
            result = events.accept(), if event_workers.len() < 16 => {
                match result {
                    Ok((stream, _)) => { event_workers.spawn(serve_probe(stream, access.clone(), GuardianChannel::Events, owner, management.clone())); }
                    Err(error) => break Err(error.into()),
                }
            }
            result = io_listener.accept(), if io_workers.len() < 16 => {
                match result {
                    Ok((stream, _)) => { io_workers.spawn(serve_probe(stream, access.clone(), GuardianChannel::Io, owner, management.clone())); }
                    Err(error) => break Err(error.into()),
                }
            }
        }
    };
    control_workers.shutdown().await;
    event_workers.shutdown().await;
    io_workers.shutdown().await;
    // Stable lock and endpoints are not unlinked on shutdown; old scope initialization stays closed.
    result
}

/// Rejects a peer before returning any facts; sessions bind the scope and socket role.
async fn serve_probe(
    mut stream: UnixStream,
    access: GuardianAccess,
    channel: GuardianChannel,
    owner: u32,
    management: Arc<Mutex<Management>>,
) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(/*secs*/ 5), async {
        if stream.peer_cred()?.uid() != owner {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "guardian peer identity mismatch",
            ));
        }
        let request: GuardianRequest = read_message(&mut stream).await?;
        let (version, intent, requested_channel) = match &request {
            GuardianRequest::Ready(request) => (request.version, &request.intent, request.channel),
            GuardianRequest::Management(request) => {
                (request.version, &request.intent, request.channel)
            }
            GuardianRequest::Run(request) => (request.version, &request.intent, request.channel),
        };
        if version != GUARDIAN_WIRE_VERSION
            || intent != &access.intent
            || requested_channel != channel
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "guardian probe rejected",
            ));
        }
        let frame = match request {
            GuardianRequest::Run(request) => {
                let reply = management.lock().await.execute_run(channel, request);
                encode_guardian_frame(&reply)?
            }
            GuardianRequest::Ready(request) => encode_guardian_frame(&GuardianReady {
                version: GUARDIAN_WIRE_VERSION,
                intent: access.intent,
                channel,
                session: request.session,
            })?,
            GuardianRequest::Management(request) => {
                // Decode/queue before locking; check the live binding only when executing. No await
                // separates the authority check from its SQLite effect or the response fact.
                let reply = management.lock().await.execute(channel, request.operation);
                encode_guardian_frame(&reply)?
            }
        };
        stream.write_all(&frame).await?;
        Ok(())
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "guardian probe timed out"))?
}

/// Bounds allocation before decoding; the caller supplies the deadline for the whole exchange.
async fn read_message<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> io::Result<T> {
    let length = stream.read_u32().await? as usize;
    if length > GUARDIAN_MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guardian frame exceeds limit",
        ));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    decode_guardian_payload(&bytes)
}
