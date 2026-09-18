use std::{future::Future, io, path::PathBuf, sync::Arc, time::Duration};

use ora_process_protocol::{
    GUARDIAN_MAX_FRAME, GUARDIAN_OUTPUT_CHUNK_LIMIT, GuardianRunOperation, GuardianRunResult,
    HOST_WIRE_VERSION, HostOperation, HostRejection, HostReply, HostRequest,
    decode_guardian_payload, encode_guardian_frame,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::Mutex,
    task::JoinSet,
};

use crate::{HostCoordinator, HostState, ProcessStateError};

/// Serves durable trusted-local responsibility independently of Node connections and lifetime.
/// Shutdown stops host coordination only; explicit Scope close owns workload termination.
pub async fn serve_process_host(
    state: HostState,
    guardian: PathBuf,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ProcessStateError> {
    let (control, output) = state.bind_endpoints().await?;
    // SAFETY: geteuid reads the current process identity without mutation.
    let owner = unsafe { libc::geteuid() };
    let host = Arc::new(Mutex::new(HostCoordinator::new(state, guardian)));
    let mut control_workers = JoinSet::new();
    let mut output_workers = JoinSet::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(/*millis*/ 50));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tokio::pin!(shutdown);
    let result = loop {
        tokio::select! {
            () = &mut shutdown => break Ok(()),
            _ = ticker.tick() => {
                if let Err(error) = host.lock().await.tick() { break Err(error); }
            }
            result = control_workers.join_next(), if !control_workers.is_empty() => {
                if let Some(Err(error)) = result { break Err(io::Error::other(error).into()); }
            }
            result = output_workers.join_next(), if !output_workers.is_empty() => {
                if let Some(Err(error)) = result { break Err(io::Error::other(error).into()); }
            }
            result = control.accept(), if control_workers.len() < 16 => {
                match result {
                    Ok((stream, _)) => { control_workers.spawn(serve_one(stream, "host.sock", owner, host.clone())); }
                    Err(error) => break Err(error.into()),
                }
            }
            result = output.accept(), if output_workers.len() < 16 => {
                match result {
                    Ok((stream, _)) => { output_workers.spawn(serve_one(stream, "host-io.sock", owner, host.clone())); }
                    Err(error) => break Err(error.into()),
                }
            }
        }
    };
    control_workers.shutdown().await;
    output_workers.shutdown().await;
    // No endpoint or journal is erased. Recovery owns stale socket replacement under the same lock.
    result
}

/// Bounded peers cannot keep admission locks while reading frames or waiting for guardian output.
async fn serve_one(
    mut stream: UnixStream,
    endpoint: &'static str,
    owner: u32,
    host: Arc<Mutex<HostCoordinator>>,
) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(/*secs*/ 5), async {
        if stream.peer_cred()?.uid() != owner {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "host peer identity mismatch",
            ));
        }
        let length = stream.read_u32().await? as usize;
        if length > GUARDIAN_MAX_FRAME {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "host frame exceeds limit",
            ));
        }
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).await?;
        let request: HostRequest = decode_guardian_payload(&bytes)?;
        let reply = if request.version != HOST_WIRE_VERSION {
            HostReply::Rejected(HostRejection::IncompatibleVersion)
        } else if request.operation.socket_name() != endpoint {
            HostReply::Rejected(HostRejection::IntentRejected)
        } else {
            execute(&host, request.operation).await
        };
        stream.write_all(&encode_guardian_frame(&reply)?).await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "host exchange timed out"))?
}

/// Mutations commit before response delivery; a vanished waiter cannot cancel accepted responsibility.
async fn execute(host: &Mutex<HostCoordinator>, operation: HostOperation) -> HostReply {
    if let HostOperation::Output {
        run,
        stream,
        offset,
        max_bytes,
    } = operation
    {
        if max_bytes > GUARDIAN_OUTPUT_CHUNK_LIMIT {
            return HostReply::Rejected(HostRejection::IntentRejected);
        }
        let client = match host.lock().await.output_client(run) {
            Ok(client) => client,
            Err(error) => return reject(error),
        };
        return match client
            .execute(GuardianRunOperation::Output {
                run,
                stream,
                offset,
                max_bytes,
            })
            .await
        {
            Ok(GuardianRunResult::Output {
                run,
                stream,
                offset,
                output,
            }) => HostReply::Output {
                run,
                stream,
                offset,
                output,
            },
            Ok(GuardianRunResult::Rejected(reason)) => {
                HostReply::Rejected(HostRejection::Guardian(reason))
            }
            Ok(GuardianRunResult::Run(_) | GuardianRunResult::Scope(_)) | Err(_) => {
                HostReply::Rejected(HostRejection::GuardianUnavailable)
            }
        };
    }
    let mut host = host.lock().await;
    let result = match operation {
        HostOperation::Inspect => Ok(HostReply::Ready(host.binding())),
        HostOperation::CreateScope { scope } => host.create_scope(scope).map(HostReply::Scope),
        HostOperation::Start { intent } => host.start(intent).map(HostReply::Run),
        HostOperation::QueryRun { run } => host.query_run(run).map(HostReply::Run),
        HostOperation::Stop { run } => host.stop(run).map(HostReply::Run),
        HostOperation::Close { scope } => host.close(scope).map(HostReply::Scope),
        HostOperation::QueryScope { scope } => host.query_scope(scope).map(HostReply::Scope),
        HostOperation::Output { .. } => unreachable!("output exchanged outside host lock"),
    };
    result.unwrap_or_else(reject)
}

/// Internal errors never disclose private launch specifications or imply safe automatic re-execution.
fn reject(error: ProcessStateError) -> HostReply {
    HostReply::Rejected(match error {
        ProcessStateError::Rejected("unknown host Run" | "unknown host Scope") => {
            HostRejection::UnknownIdentity
        }
        ProcessStateError::Rejected(_) => HostRejection::IntentRejected,
        ProcessStateError::Identity(_) => HostRejection::IntentRejected,
        ProcessStateError::Io(_) | ProcessStateError::Sqlite(_) => {
            HostRejection::StorageUnavailable
        }
    })
}
