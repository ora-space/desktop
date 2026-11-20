//! Root-only local reconciliation shares the running Node's journal and never opens a second DB.
use super::{files::StopFiles, now_ms};
use crate::{SessionStopConfig, managed::attempt_lock::StopEvidence};
use ora_node_db::*;
use serde::{Deserialize, Serialize};
use std::{io, os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixListener,
    sync::watch,
    task::JoinHandle,
};

const SOCKET: &str = "session-stop.sock";

/// Root-only names needed to address an original retained attempt during reconciliation.
#[derive(Debug, Deserialize, Serialize)]
pub struct ProbeResidue {
    execution: ora_node_protocol::ExecutionId,
    attempt: AttemptId,
    state: SessionStopState,
    bound_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    ProbeCapacity {
        total: usize,
        running: usize,
        cleanup_pending: usize,
        queued: usize,
        available: usize,
        residues: Vec<ProbeResidue>,
    },
    Completed {
        result: SessionStopAuditResult,
    },
    InProgress,
    Rejected {
        code: String,
    },
}

/// A running listener owns its socket until all admitted audit operations have committed results.
pub(crate) struct Admin {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Admin {
    /// Private Node home and SO_PEERCRED both enforce the local root boundary.
    pub(crate) fn start(
        home: &Path,
        config: SessionStopConfig,
        journal: SessionJournal,
        probes: crate::session::mcp::probes::Scheduler,
    ) -> io::Result<Self> {
        let path = home.join(SOCKET);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                use std::os::unix::fs::FileTypeExt;
                if !meta.file_type().is_socket() {
                    return Err(io::Error::other("invalid session stop socket"));
                }
                std::fs::remove_file(&path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let files = StopFiles::open(home)?;
        let listener = UnixListener::from_std(listener)?;
        let interrupted = journal
            .incomplete_session_stop_audits()
            .map_err(io::Error::other)?;
        let (stop, mut stopping) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut pending = tokio::task::JoinSet::new();
            // Interrupted requests need fresh evidence; processing is bounded and does not replay commands.
            let mut interrupted = std::collections::VecDeque::from(interrupted);
            let mut gc = tokio::time::interval(Duration::from_secs(3600));
            loop {
                while pending.len() < 8 {
                    let Some(request) = interrupted.pop_front() else {
                        break;
                    };
                    let journal = journal.clone();
                    let files = files.clone();
                    let config = config.clone();
                    let probes = probes.clone();
                    pending.spawn(async move {
                        let _ = reconcile(&journal, &files, &config, &request).await;
                        probes.1.notify_waiters();
                    });
                }
                tokio::select! {
                    _=stopping.wait_for(|stop|*stop) => break,
                    _=gc.tick() => { collect(&journal,&files); },
                    Some(_)=pending.join_next(), if !pending.is_empty() => {},
                    accepted=listener.accept(), if pending.len()<8 => {
                        let Ok((stream,_))=accepted else { break; };
                        let journal=journal.clone(); let files=files.clone(); let config=config.clone(); let probes=probes.clone();
                        pending.spawn(async move {
                            // UnixStream::peer_cred reads SO_PEERCRED; the payload has no actor identity.
                            let Ok(peer)=stream.peer_cred() else { return; };
                            if peer.uid()!=0 { return; }
                            let (read,mut write)=stream.into_split();
                            let mut line=String::new();
                            let read=tokio::time::timeout(Duration::from_secs(2),BufReader::new(read).take(4097).read_line(&mut line)).await;
                            if !matches!(read,Ok(Ok(1..=4096))) || !line.ends_with('\n') { return; }
                            let capacity = serde_json::from_str::<serde_json::Value>(&line).ok() == Some(serde_json::json!({"operation":"probe_capacity"}));
                            let reply=if capacity { capacity_summary(&journal, &probes) } else { match serde_json::from_str::<SessionStopReconcile>(&line) {
                                Ok(request) => match journal.begin_session_stop_audit(peer.uid(),&request,now_ms()) {
                                    Ok(SessionStopAuditAdmission::Completed(result)) => Reply::Completed{result},
                                    Ok(SessionStopAuditAdmission::InProgress) => Reply::InProgress,
                                    Ok(SessionStopAuditAdmission::Started) => match reconcile(&journal,&files,&config,&request).await {
                                        Ok(result) => Reply::Completed{result},
                                        Err(_) => rejected("audit_unavailable"),
                                    },
                                    Err(Error::IdentityConflict) => rejected("request_conflict"),
                                    Err(_) => rejected("invalid_request"),
                                },
                                Err(_) => rejected("invalid_request"),
                            }};
                            if matches!(reply, Reply::Completed { .. }) { probes.1.notify_waiters(); }
                            if let Ok(mut bytes)=serde_json::to_vec(&reply) {
                                bytes.push(b'\n');
                                let _=tokio::time::timeout(Duration::from_secs(2),write.write_all(&bytes)).await;
                            }
                        });
                    }
                }
            }
            while pending.join_next().await.is_some() {}
            let _ = std::fs::remove_file(path);
        });
        Ok(Self { stop, task })
    }

    /// Finishes admitted audits before the Node releases its exclusive journal lease.
    pub(crate) async fn close(self) {
        self.stop.send_replace(true);
        let _ = self.task.await;
    }
}

/// Counts remain anonymous outside this root-only response; residues contain no settings.
fn capacity_summary(
    journal: &SessionJournal,
    probes: &crate::session::mcp::probes::Scheduler,
) -> Reply {
    let Ok(records) = journal.session_stop_records(None) else {
        return rejected("journal_unavailable");
    };
    let mut reply = probes.summary();
    if let Reply::ProbeCapacity { residues, .. } = &mut reply {
        *residues = records
            .into_iter()
            .filter(|record| {
                !record.resources_released
                    && matches!(
                        record.identity.kind,
                        SessionStopKind::StdioProbe | SessionStopKind::HttpProbe
                    )
            })
            .map(|record| ProbeResidue {
                execution: record.identity.execution,
                attempt: record.identity.attempt,
                state: record.state,
                bound_at_ms: record.bound_at_ms,
            })
            .collect();
    }
    reply
}

/// The client receives only stable codes, never a database error or process command line.
fn rejected(code: &str) -> Reply {
    Reply::Rejected { code: code.into() }
}

/// Observes for at most ten seconds and commits the audit and confirmation together.
async fn reconcile(
    journal: &SessionJournal,
    files: &StopFiles,
    config: &SessionStopConfig,
    request: &SessionStopReconcile,
) -> Result<SessionStopAuditResult, Error> {
    let record = journal
        .session_stop_records(Some(&request.execution))?
        .into_iter()
        .find(|record| record.identity.attempt == request.attempt)
        .ok_or(Error::InvalidTransition)?;
    let bound = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut pause = Duration::from_millis(25);
    let (evidence, proof) = loop {
        if record.state == SessionStopState::Confirmed {
            break (StopEvidence::Stopped, record.proof);
        }
        if record.state == SessionStopState::Prepared {
            break (StopEvidence::Unknown, None);
        }
        let (evidence, proof) = files
            .observe(
                &record.identity,
                record.scope,
                &config.host(),
                config.single_mount_with_storage_fencing,
            )
            .await;
        let allowed = proof.filter(|proof| admits(request.mode, *proof));
        if allowed.is_some() {
            break (StopEvidence::Stopped, allowed);
        }
        if request.mode == SessionStopMode::AfterHostReboot
            || evidence == StopEvidence::Unknown
            || tokio::time::Instant::now() >= bound
        {
            break (
                if evidence == StopEvidence::Stopped {
                    StopEvidence::Unknown
                } else {
                    evidence
                },
                None,
            );
        }
        files
            .terminate_off_runtime(&record.identity, &config.host())
            .await;
        tokio::time::sleep(pause).await;
        pause = (pause * 2).min(Duration::from_secs(1));
    };
    let result = SessionStopAuditResult {
        observation: match evidence {
            StopEvidence::Stopped => SessionStopObservation::Stopped,
            StopEvidence::Running => SessionStopObservation::Running,
            StopEvidence::Unknown => SessionStopObservation::Unknown,
        },
        before: record.state,
        after: if proof.is_some() {
            SessionStopState::Confirmed
        } else {
            record.state
        },
        proof,
        released: if record.resources_released {
            record.identity.packages
        } else {
            vec![]
        },
    };
    journal.finish_session_stop_audit(request, &result, now_ms())?;
    Ok(result)
}

/// Whether a reconciliation mode accepts a proof. Both same-boot proofs, with or without the
/// mark scan, show only that this boot's processes are gone, never that the host rebooted.
fn admits(mode: SessionStopMode, proof: SessionStopProof) -> bool {
    match mode {
        SessionStopMode::Recheck => true,
        SessionStopMode::AfterHostReboot => match proof {
            SessionStopProof::LockAndProcesses | SessionStopProof::LockProcessesAndMarks => false,
            SessionStopProof::HostReboot | SessionStopProof::FencedVolume => true,
        },
    }
}

/// Confirmed details retire in bounded hourly batches; unconfirmed rows never age out.
fn collect(journal: &SessionJournal, files: &StopFiles) {
    let now = now_ms();
    if let Ok(attempts) = journal.session_stop_gc_candidates(now) {
        for attempt in attempts {
            if files.retire(&attempt).is_ok() {
                let _ = journal.retire_session_stop(&attempt, now);
            }
        }
    }
}

/// A management CLI is only a bounded Unix-socket client; it cannot mutate a stopped Node's DB.
pub fn client(home: &Path, request: &SessionStopReconcile) -> io::Result<Reply> {
    use std::io::{BufRead, Read, Write};
    if !home.is_absolute() {
        return Err(io::Error::other("Node home must be absolute"));
    }
    let mut stream = std::os::unix::net::UnixStream::connect(home.join(SOCKET))?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    let mut response = String::new();
    std::io::BufReader::new(stream)
        .take(65537)
        .read_line(&mut response)?;
    if response.len() > 65536 || !response.ends_with('\n') {
        return Err(io::Error::other("invalid reconciliation response"));
    }
    serde_json::from_str(&response).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::admits;
    use ora_node_db::{SessionStopMode, SessionStopProof};
    use pretty_assertions::assert_eq;

    /// After-host-reboot reconciliation accepts only reboot or fencing evidence; recheck accepts
    /// whatever proof the original evidence produced.
    #[test]
    fn after_host_reboot_refuses_both_same_boot_proofs() {
        let proofs = [
            SessionStopProof::LockAndProcesses,
            SessionStopProof::LockProcessesAndMarks,
            SessionStopProof::HostReboot,
            SessionStopProof::FencedVolume,
        ];
        assert_eq!(
            (
                proofs.map(|proof| admits(SessionStopMode::Recheck, proof)),
                proofs.map(|proof| admits(SessionStopMode::AfterHostReboot, proof)),
            ),
            ([true; 4], [false, false, true, true])
        );
    }
}
