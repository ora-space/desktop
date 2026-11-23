//! Session-scoped OFD evidence, one identity-checked clue for every host-spawned process group,
//! and the attempt mark that reaches processes outside those groups.
use super::marks;
use crate::managed::attempt_lock::{AttemptLocks, LockDurability, StopEvidence};
use ora_node_db::{
    AttemptId, SessionStopHost, SessionStopIdentity, SessionStopKind, SessionStopProof,
    SessionStopScope,
};
use ora_process::ProcessIdentity;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read, Write},
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Stop observations of every session run one at a time on one blocking worker per Node, never
/// on the Node's single runtime thread: a scan walks all of procfs, and one `environ` read can
/// hang on a stuck filesystem.
static STOP_WORKER: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// How long one stop observation, including waiting for the worker, may take before it counts as
/// Unknown. The wait counts on purpose: a hung read keeps the worker, and only a budget that
/// includes the wait makes later observations give up instead of queueing behind it forever, or
/// piling up threads if they did not queue at all.
pub(crate) const STOP_WORK_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// The budget of an observation that must also end by `deadline`.
pub(crate) fn budget_until(deadline: tokio::time::Instant) -> std::time::Duration {
    deadline
        .saturating_duration_since(tokio::time::Instant::now())
        .min(STOP_WORK_BUDGET)
}

/// Runs one blocking stop observation on the worker; `None` when it did not finish in budget.
async fn off_runtime<T: Send + 'static>(
    budget: std::time::Duration,
    job: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    off_runtime_within(&STOP_WORKER, budget, job).await
}

/// As [`off_runtime`] on an explicit worker. The job keeps the worker until it returns, even
/// after its caller gave up, so a hung read makes later observations time out too. A spent
/// budget starts no job at all.
pub(super) async fn off_runtime_within<T: Send + 'static>(
    worker: &'static tokio::sync::Semaphore,
    budget: std::time::Duration,
    job: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    if budget.is_zero() {
        return None;
    }
    tokio::time::timeout(budget, async {
        let permit = worker.acquire().await.ok()?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            job()
        })
        .await
        .ok()
    })
    .await
    .ok()
    .flatten()
}

#[derive(Clone)]
pub(crate) struct StopFiles {
    locks: AttemptLocks,
    directory: PathBuf,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Clue {
    pid: u32,
    start_ticks: u64,
    boot_id: String,
}

impl StopFiles {
    /// Reuses Git's lock mechanics in a separate private directory and naming namespace.
    pub(crate) fn open(home: &Path) -> io::Result<Self> {
        Ok(Self {
            locks: AttemptLocks::open_sessions(home)?,
            directory: home.join("session-attempts"),
        })
    }

    /// The lock marker, the mark scope of processes running as `identity`, and their parent
    /// directory are durable before the database takes responsibility.
    pub(crate) fn create(&self, attempt: &AttemptId, identity: ProcessIdentity) -> io::Result<()> {
        self.locks.create(
            attempt,
            Path::new(attempt.as_str()),
            LockDurability::Recorded,
        )?;
        marks::record(&self.directory, attempt, identity)
    }

    /// Acquires after recording; the owner serializes this against recovery and dispatch.
    pub(crate) fn hold(&self, attempt: &AttemptId) -> io::Result<Arc<OwnedFd>> {
        self.locks.hold(attempt)
    }

    /// An observed clue-write failure is retained conservatively; it is distinct from no dispatch.
    pub(crate) fn mark_incomplete(&self, attempt: &AttemptId) -> io::Result<()> {
        let path = self.directory.join(format!("{attempt}.incomplete.clue"));
        let mut file = fs::File::create(&path)?;
        file.write_all(b"incomplete")?;
        file.sync_all()?;
        fs::File::open(&self.directory)?.sync_all()
    }

    /// Clues name processes only; argument vectors, environment and Settings never enter files.
    pub(crate) fn note_child(
        &self,
        attempt: &AttemptId,
        host: &SessionStopHost,
        pid: u32,
    ) -> io::Result<()> {
        let process = match ora_utils::process::linux_process(pid) {
            Ok(process) => process,
            Err(error) if error.kind() == io::ErrorKind::NotFound && !group_exists(pid) => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let clue = Clue {
            pid,
            start_ticks: process.start_ticks,
            boot_id: host.boot_id.clone(),
        };
        let mut file = fs::File::create(self.directory.join(format!("{attempt}.{pid}.clue")))?;
        file.write_all(&serde_json::to_vec(&clue)?)?;
        file.sync_all()?;
        fs::File::open(&self.directory)?.sync_all()
    }

    /// A different kernel needs affirmative deployment evidence; an idle local lock is insufficient.
    pub(crate) fn probe(
        &self,
        identity: &SessionStopIdentity,
        scope: SessionStopScope,
        current: &SessionStopHost,
        fenced_volume: bool,
    ) -> (StopEvidence, Option<SessionStopProof>) {
        if !identity.host.boot_id.is_empty()
            && !current.boot_id.is_empty()
            && identity.host.boot_id != current.boot_id
        {
            if identity
                .host
                .trusted_host_id
                .as_ref()
                .is_some_and(|id| !id.is_empty())
                && identity.host.trusted_host_id == current.trusted_host_id
            {
                return (StopEvidence::Stopped, Some(SessionStopProof::HostReboot));
            }
            if fenced_volume {
                return (StopEvidence::Stopped, Some(SessionStopProof::FencedVolume));
            }
            return (StopEvidence::Unknown, None);
        }
        if identity.host.boot_id.is_empty()
            || identity.host.boot_id != current.boot_id
            || identity.kind == SessionStopKind::LegacyAgent
        {
            return (StopEvidence::Unknown, None);
        }
        let lock = self.directory.join(format!("{}.lock", identity.attempt));
        let valid_marker = || -> io::Result<bool> {
            if !fs::symlink_metadata(&lock)?.is_file() {
                return Ok(false);
            }
            let mut marker = Vec::new();
            fs::File::open(&lock)?.take(128).read_to_end(&mut marker)?;
            Ok(marker == identity.attempt.as_str().as_bytes())
        };
        if !matches!(valid_marker(), Ok(true)) {
            return (StopEvidence::Unknown, None);
        }
        let evidence = self.locks.probe(&identity.attempt);
        if evidence != StopEvidence::Stopped {
            return (evidence, None);
        }
        let clues = match self.clues(&identity.attempt) {
            Ok(clues) => clues,
            Err(_) => return (StopEvidence::Unknown, None),
        };
        for clue in clues {
            if clue.boot_id != current.boot_id {
                return (StopEvidence::Unknown, None);
            }
            match ora_utils::process::linux_process(clue.pid) {
                Ok(process) if process.start_ticks == clue.start_ticks => {
                    return (StopEvidence::Running, None);
                }
                Ok(_) => {} // Reused PID is not our process; never signal it.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if group_exists(clue.pid) {
                        return (StopEvidence::Running, None);
                    }
                }
                Err(_) => return (StopEvidence::Unknown, None),
            }
        }
        // Jobs that left the original groups and dropped the inherited descriptor stay visible
        // only through the mark. The proof names which evidence held, so an audit can tell a
        // marked attempt from one registered before marks existed.
        let proof = match scope {
            SessionStopScope::Absent => SessionStopProof::LockAndProcesses,
            SessionStopScope::Recorded => match marks::scope(&self.directory, &identity.attempt) {
                Ok(Some(mark)) => match marks::evidence(|| mark.scan()) {
                    StopEvidence::Stopped => SessionStopProof::LockProcessesAndMarks,
                    evidence @ (StopEvidence::Running | StopEvidence::Unknown) => {
                        return (evidence, None);
                    }
                },
                // Registered with a scope that is now missing or damaged: nothing shows that the
                // attempt's marked jobs are gone, and the weaker proof would hide that.
                Ok(None) | Err(_) => return (StopEvidence::Unknown, None),
            },
        };
        (StopEvidence::Stopped, Some(proof))
    }

    /// Signals only processes of the original attempt: those its mark claims, and the groups
    /// whose boot and direct-child identity still match.
    pub(crate) fn terminate(&self, identity: &SessionStopIdentity, current: &SessionStopHost) {
        if current.boot_id.is_empty() || current.boot_id != identity.host.boot_id {
            return;
        }
        // Marked processes are frozen and killed before any group is signalled: a group kill
        // would orphan an environment-cleared child before the scan sees its marked parent.
        // Group members carry the mark too; the group kill still reaches those that cleared it.
        if let Ok(Some(scope)) = marks::scope(&self.directory, &identity.attempt) {
            let _ = scope.kill();
        }
        let Ok(clues) = self.clues(&identity.attempt) else {
            return;
        };
        for clue in clues {
            if clue.boot_id != current.boot_id {
                continue;
            }
            match ora_utils::process::linux_process(clue.pid) {
                Ok(process) if process.start_ticks != clue.start_ticks => continue,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => continue,
            }
            if let Ok(pid) = i32::try_from(clue.pid) {
                // SAFETY: only an identity-checked original process group is addressed.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
    }

    /// Terminates the attempt and then probes it, off the runtime; Unknown when out of budget.
    pub(crate) async fn settle(
        &self,
        identity: &SessionStopIdentity,
        scope: SessionStopScope,
        current: &SessionStopHost,
        fenced_volume: bool,
    ) -> (StopEvidence, Option<SessionStopProof>) {
        let (files, identity, current) = (self.clone(), identity.clone(), current.clone());
        off_runtime(STOP_WORK_BUDGET, move || {
            files.terminate(&identity, &current);
            files.probe(&identity, scope, &current, fenced_volume)
        })
        .await
        .unwrap_or((StopEvidence::Unknown, None))
    }

    /// Probes the attempt off the runtime without signalling it; Unknown when out of `budget`.
    pub(crate) async fn observe(
        &self,
        identity: &SessionStopIdentity,
        scope: SessionStopScope,
        current: &SessionStopHost,
        fenced_volume: bool,
        budget: std::time::Duration,
    ) -> (StopEvidence, Option<SessionStopProof>) {
        let (files, identity, current) = (self.clone(), identity.clone(), current.clone());
        off_runtime(budget, move || {
            files.probe(&identity, scope, &current, fenced_volume)
        })
        .await
        .unwrap_or((StopEvidence::Unknown, None))
    }

    /// Terminates the attempt off the runtime; a request only, so an expired `budget` is silent.
    pub(crate) async fn terminate_off_runtime(
        &self,
        identity: &SessionStopIdentity,
        current: &SessionStopHost,
        budget: std::time::Duration,
    ) {
        let (files, identity, current) = (self.clone(), identity.clone(), current.clone());
        off_runtime(budget, move || files.terminate(&identity, &current)).await;
    }

    /// Called only after durable confirmation and age/reference checks, never to manufacture proof.
    pub(crate) fn retire(&self, attempt: &AttemptId) -> io::Result<()> {
        let prefix = format!("{attempt}.");
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
            {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    /// Corrupt/unreadable clues keep the attempt unknown; missing optional clues do not replace locks.
    fn clues(&self, attempt: &AttemptId) -> io::Result<Vec<Clue>> {
        let prefix = format!("{attempt}.");
        let mut clues = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".clue"))
            {
                if !entry.file_type()?.is_file() {
                    return Err(io::Error::other("invalid process clue"));
                }
                let mut bytes = Vec::new();
                fs::File::open(entry.path())?
                    .take(1024)
                    .read_to_end(&mut bytes)?;
                clues.push(
                    serde_json::from_slice(&bytes)
                        .map_err(|_| io::Error::other("invalid process clue"))?,
                );
            }
        }
        Ok(clues)
    }
}

/// A leader may already be reaped while one of its original same-group descendants remains.
fn group_exists(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    // SAFETY: signal zero observes existence and does not terminate a process.
    unsafe {
        libc::kill(-pid, 0) == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}
