//! Independent process responsibility outlives conversations, plugin handles, and Node restarts.
pub(crate) mod admin;
pub(crate) mod files;
pub(crate) mod spawner;
#[cfg(test)]
mod tests;

use super::{SessionLedger, workload::SessionPlacement};
use crate::{SessionStopConfig, managed::attempt_lock::StopEvidence};
use files::StopFiles;
use ora_node_db::{AttemptId, SessionStopChange, SessionStopIdentity, SessionStopState};
use ora_node_protocol::ExecutionId;
use ora_process::TokioProcessSpawner;
use spawner::SessionAttemptSpawner;
use std::{io, path::Path, time::Duration};

/// Resources stay owned after the actor hands an unconfirmed attempt to background reconciliation.
pub(super) struct Retained<T> {
    pub placement: SessionPlacement,
    pub leases: Vec<T>,
    pub spawner: SessionAttemptSpawner,
    pub isolation: super::process::ModelProcessIsolation,
}

/// Wall-clock deadlines survive Node restart and are never extended by subsequent stop requests.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// A failed lifecycle request must take the same bounded evidence path as a successful request.
pub(super) async fn await_plugin_stop<L: SessionLedger, T, E: std::fmt::Display>(
    ledger: &L,
    home: &Path,
    config: &SessionStopConfig,
    execution: &ExecutionId,
    stop: impl std::future::Future<Output = Result<T, E>>,
) -> Result<bool, L::Error> {
    if let Err(error) = stop.await {
        ora_logging::ora_warn!(execution_id = %execution, error = %error, "agent plugin did not stop cleanly");
    }
    // Success and failure are both requests, never proof; the original deadline still governs.
    confirm(ledger, home, config, execution, true).await
}

/// Lock creation, durable registration and taking the OFD all precede the first dispatch.
pub(super) fn prepare<L: SessionLedger>(
    ledger: &L,
    home: &Path,
    identity: SessionStopIdentity,
    inner: TokioProcessSpawner,
) -> io::Result<SessionAttemptSpawner> {
    let files = StopFiles::open(home)?;
    files.create(&identity.attempt)?;
    ledger
        .record_session_stop(&identity, now_ms())
        .map_err(|_| io::Error::other("stop journal unavailable"))?;
    SessionAttemptSpawner::new(inner, identity, files)
}

/// Anchors all outstanding responsibilities before notifying any process or closing admission.
pub(super) fn request<L: SessionLedger>(
    ledger: &L,
    execution: &ExecutionId,
    config: &SessionStopConfig,
) -> Result<(), L::Error> {
    let at_ms = now_ms();
    for record in ledger.session_stop_records(Some(execution))? {
        if record.state != SessionStopState::Confirmed {
            ledger.change_session_stop(
                &record.identity.attempt,
                SessionStopChange::Request {
                    at_ms,
                    bound_at_ms: at_ms.saturating_add(config.confirmation_timeout_ms as i64),
                },
            )?;
        }
    }
    Ok(())
}

/// Proves original attempts only; a missing file or failed read leaves ownership unchanged.
pub(crate) async fn confirm<L: SessionLedger>(
    ledger: &L,
    home: &Path,
    config: &SessionStopConfig,
    execution: &ExecutionId,
    wait: bool,
) -> Result<bool, L::Error> {
    request(ledger, execution, config)?;
    let files = StopFiles::open(home).ok();
    loop {
        let mut pending = false;
        let mut before_bound = false;
        for record in ledger.session_stop_records(Some(execution))? {
            if record.state == SessionStopState::Confirmed {
                continue;
            }
            let proof = files.as_ref().and_then(|files| {
                files.terminate(&record.identity, &config.host());
                let (evidence, proof) = files.probe(
                    &record.identity,
                    &config.host(),
                    config.single_mount_with_storage_fencing,
                );
                (evidence == StopEvidence::Stopped)
                    .then_some(proof)
                    .flatten()
            });
            if let Some(proof) = proof {
                ledger.change_session_stop(
                    &record.identity.attempt,
                    SessionStopChange::Confirm {
                        at_ms: now_ms(),
                        proof,
                    },
                )?;
            } else {
                pending = true;
                if record.bound_at_ms.is_some_and(|bound| now_ms() < bound) {
                    before_bound = true;
                } else if record.state != SessionStopState::StopUnconfirmed {
                    ledger.change_session_stop(
                        &record.identity.attempt,
                        SessionStopChange::Unconfirmed,
                    )?;
                }
            }
        }
        if !pending {
            return Ok(true);
        }
        if !wait || !before_bound {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Resource release is a second durable boundary, after proof and actual removal of local views.
pub(crate) fn released<L: SessionLedger>(
    ledger: &L,
    execution: &ExecutionId,
) -> Result<(), L::Error> {
    for record in ledger
        .session_stop_records(Some(execution))?
        .into_iter()
        .filter(|r| r.state == SessionStopState::Confirmed && !r.resources_released)
    {
        ledger.change_session_stop(
            &record.identity.attempt,
            SessionStopChange::ResourcesReleased,
        )?;
    }
    Ok(())
}

/// Generates an opaque original attempt identity; neither execution nor package paths are reused.
pub(super) fn attempt_id() -> AttemptId {
    AttemptId::generate()
}
