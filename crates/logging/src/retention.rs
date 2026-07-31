use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use time::{Date, macros::format_description};

use crate::FileSystemAction;
use crate::LoggingInitError;
use crate::file_output::ActiveLogPath;
use crate::health::LoggingHealthRecorder;

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(/*millis*/ 50);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(/*secs*/ 1);

/// Performs one retention pass for the latest active file selected by the writer.
pub(crate) trait RetentionCleaner: Send + 'static {
    /// Deletes expired files while preserving paths owned by active or pending rotation.
    fn cleanup(&self, protection: &LogFileProtection) -> Result<(), LoggingInitError>;
}

/// Coordinates file deletion with active and pending rotation paths.
#[derive(Clone, Debug)]
pub(crate) struct LogFileProtection {
    shared: Arc<ProtectionShared>,
}

impl LogFileProtection {
    /// Protects the initially opened local-date file.
    pub(crate) fn new(current_log_path: PathBuf) -> Self {
        Self {
            shared: Arc::new(ProtectionShared {
                state: Mutex::new(ProtectionState {
                    current_log_path,
                    pending_rotation: PendingRotation::None,
                    deletion: DeletionState::Idle,
                }),
            }),
        }
    }

    /// Reserves a rollover target unless retention is already deleting that exact path.
    pub(crate) fn protect_rotation_target(&self, path: PathBuf) -> RotationTargetProtection {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.deletion.targets(&path) {
            return RotationTargetProtection::DeletionInProgress;
        }
        state.pending_rotation = PendingRotation::Protected(path);
        RotationTargetProtection::Protected
    }

    /// Promotes a successfully opened rollover target to the current active file.
    pub(crate) fn activate(&self, path: PathBuf) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.current_log_path = path;
        state.pending_rotation = PendingRotation::None;
    }

    /// Returns the current active path for retention diagnostics and tests.
    #[cfg(test)]
    pub(crate) fn current_log_path(&self) -> PathBuf {
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .current_log_path
            .clone()
    }

    /// Removes one unprotected candidate while reserving it against concurrent rotation.
    fn remove_if_inactive(&self, path: &Path) -> Result<RemovalOutcome, std::io::Error> {
        {
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if state.protects(path) {
                return Ok(RemovalOutcome::Protected);
            }
            state.deletion = DeletionState::Deleting(path.to_path_buf());
        }

        let result = fs::remove_file(path);
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.deletion = DeletionState::Idle;

        result.map(|()| RemovalOutcome::Removed)
    }
}

/// Schedules coalesced cleanup work without blocking the logging writer thread.
#[derive(Clone, Debug)]
pub(crate) enum RetentionHandle {
    Active {
        requests: SyncSender<()>,
        directory: PathBuf,
        health: LoggingHealthRecorder,
        protection: LogFileProtection,
    },
    Inactive,
}

impl RetentionHandle {
    /// Protects a path before rotation attempts to open or append to it.
    pub(crate) fn protect_rotation_target(&self, path: PathBuf) -> RotationTargetProtection {
        match self {
            Self::Active { protection, .. } => protection.protect_rotation_target(path),
            Self::Inactive => RotationTargetProtection::Protected,
        }
    }

    /// Makes a successfully opened rollover path the sole active-file protection.
    pub(crate) fn activate(&self, path: PathBuf) {
        match self {
            Self::Active { protection, .. } => protection.activate(path),
            Self::Inactive => {}
        }
    }

    /// Wakes the cleaner unless a coalesced retention pass is already queued.
    pub(crate) fn schedule(&self) {
        match self {
            Self::Active {
                requests,
                directory,
                health,
                ..
            } => match requests.try_send(()) {
                Ok(()) | Err(TrySendError::Full(())) => {}
                Err(TrySendError::Disconnected(())) => health.record_retention_failure_message(
                    directory.clone(),
                    "retention worker stopped unexpectedly",
                ),
            },
            Self::Inactive => {}
        }
    }
}

/// Owns the background cleanup thread for the same lifetime as its file writer.
#[derive(Debug)]
pub(crate) enum RetentionWorkerGuard {
    Active {
        shutdown: mpsc::Sender<()>,
        stopped: Mutex<Receiver<()>>,
        thread: Option<JoinHandle<()>>,
    },
    Inactive,
}

impl Drop for RetentionWorkerGuard {
    fn drop(&mut self) {
        let Self::Active {
            shutdown,
            stopped,
            thread,
        } = self
        else {
            return;
        };

        let _ = shutdown.send(());
        if stopped
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(WORKER_SHUTDOWN_TIMEOUT)
            .is_ok()
            && let Some(thread) = thread.take()
        {
            let _ = thread.join();
        }
        // A filesystem call can block indefinitely on some platforms. Dropping the join handle
        // after the timeout detaches that cleanup rather than delaying application shutdown.
    }
}

/// Couples the scheduling handle with the guard that owns its worker lifetime.
#[derive(Debug)]
pub(crate) struct RetentionRuntime {
    pub(crate) handle: RetentionHandle,
    pub(crate) guard: RetentionWorkerGuard,
}

/// Starts a retention worker with an injectable cleaner for deterministic blocking and failure tests.
pub(crate) fn start_retention_worker_with<C>(
    cleaner: C,
    directory: PathBuf,
    protection: LogFileProtection,
    health: LoggingHealthRecorder,
) -> RetentionRuntime
where
    C: RetentionCleaner,
{
    let (request_sender, request_receiver) = mpsc::sync_channel(/*bound*/ 1);
    let (shutdown_sender, shutdown_receiver) = mpsc::channel();
    let (stopped_sender, stopped_receiver) = mpsc::channel();
    let worker_protection = protection.clone();
    let worker_health = health.clone();
    let worker_directory = directory.clone();
    let thread = thread::Builder::new()
        .name("ora-log-retention".to_string())
        .spawn(move || {
            run_retention_worker(
                cleaner,
                request_receiver,
                shutdown_receiver,
                stopped_sender,
                worker_protection,
                worker_directory,
                worker_health,
            );
        });

    match thread {
        Ok(thread) => RetentionRuntime {
            handle: RetentionHandle::Active {
                requests: request_sender,
                directory,
                health,
                protection,
            },
            guard: RetentionWorkerGuard::Active {
                shutdown: shutdown_sender,
                stopped: Mutex::new(stopped_receiver),
                thread: Some(thread),
            },
        },
        Err(error) => {
            health.record_retention_failure(directory, &error);
            RetentionRuntime {
                handle: RetentionHandle::Inactive,
                guard: RetentionWorkerGuard::Inactive,
            }
        }
    }
}

/// Processes coalesced cleanup notifications until shutdown or sender disconnection.
fn run_retention_worker<C>(
    cleaner: C,
    requests: Receiver<()>,
    shutdown: Receiver<()>,
    stopped: mpsc::Sender<()>,
    protection: LogFileProtection,
    directory: PathBuf,
    health: LoggingHealthRecorder,
) where
    C: RetentionCleaner,
{
    loop {
        match shutdown.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        match requests.recv_timeout(WORKER_POLL_INTERVAL) {
            Ok(()) => match cleaner.cleanup(&protection) {
                Ok(()) => health.record_retention_recovered(),
                Err(error) => health.record_retention_failure(directory.clone(), &error),
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = stopped.send(());
}

#[derive(Clone, Debug)]
pub(crate) struct FilesystemRetentionCleaner {
    active_path: ActiveLogPath,
    max_days: NonZeroUsize,
}

impl FilesystemRetentionCleaner {
    /// Creates a cleaner for one configured rotated log-file series.
    pub(crate) fn new(active_path: ActiveLogPath, max_days: NonZeroUsize) -> Self {
        Self {
            active_path,
            max_days,
        }
    }
}

impl RetentionCleaner for FilesystemRetentionCleaner {
    fn cleanup(&self, protection: &LogFileProtection) -> Result<(), LoggingInitError> {
        cleanup_old_logs(&self.active_path, self.max_days, protection)
    }
}

#[derive(Debug)]
struct ProtectionShared {
    state: Mutex<ProtectionState>,
}

#[derive(Debug)]
struct ProtectionState {
    current_log_path: PathBuf,
    pending_rotation: PendingRotation,
    deletion: DeletionState,
}

impl ProtectionState {
    /// Reports whether a candidate belongs to the active file or an in-progress rotation.
    fn protects(&self, path: &Path) -> bool {
        self.current_log_path == path || self.pending_rotation.protects(path)
    }
}

#[derive(Debug)]
enum PendingRotation {
    None,
    Protected(PathBuf),
}

impl PendingRotation {
    /// Reports whether this state protects the supplied pending rollover path.
    fn protects(&self, path: &Path) -> bool {
        match self {
            Self::None => false,
            Self::Protected(protected) => protected == path,
        }
    }
}

#[derive(Debug)]
enum DeletionState {
    Idle,
    Deleting(PathBuf),
}

impl DeletionState {
    /// Reports whether the retention worker has reserved this exact path for deletion.
    fn targets(&self, path: &Path) -> bool {
        match self {
            Self::Idle => false,
            Self::Deleting(target) => target == path,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemovalOutcome {
    Removed,
    Protected,
}

/// Reports whether rotation owns its target or should retry after an in-progress deletion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RotationTargetProtection {
    Protected,
    DeletionInProgress,
}

/// Deletes the oldest inactive files until the rotated series fits `max_days`.
pub(crate) fn cleanup_old_logs(
    active_path: &ActiveLogPath,
    max_days: NonZeroUsize,
    protection: &LogFileProtection,
) -> Result<(), LoggingInitError> {
    let directory =
        fs::read_dir(active_path.directory()).map_err(|source| LoggingInitError::FileSystem {
            action: FileSystemAction::ReadDirectory,
            path: active_path.directory().to_path_buf(),
            source,
        })?;

    let mut dated_files = directory
        .filter_map(Result::ok)
        .filter_map(|entry| parse_dated_log_file(&entry.path(), active_path))
        .collect::<Vec<_>>();
    dated_files.sort_by_key(|candidate| candidate.date);

    let files_to_delete = dated_files.len().saturating_sub(max_days.get());
    let mut deleted = 0usize;
    for candidate in dated_files {
        if deleted >= files_to_delete {
            break;
        }
        match protection.remove_if_inactive(&candidate.path) {
            Ok(RemovalOutcome::Removed) => deleted = deleted.saturating_add(1),
            Ok(RemovalOutcome::Protected) => {}
            Err(source) => {
                return Err(LoggingInitError::FileSystem {
                    action: FileSystemAction::RemoveFile,
                    path: candidate.path,
                    source,
                });
            }
        }
    }

    Ok(())
}

/// Recognizes only the files owned by one configured log-file prefix.
fn parse_dated_log_file(path: &Path, active_path: &ActiveLogPath) -> Option<DatedLogFile> {
    let file_name = path.file_name()?.to_str()?;
    let prefix = format!("{}.", active_path.file_name());

    if !file_name.starts_with(&prefix) {
        return None;
    }

    let suffix = &file_name[prefix.len()..];
    let date = Date::parse(suffix, &format_description!("[year]-[month]-[day]")).ok()?;

    Some(DatedLogFile {
        path: path.to_path_buf(),
        date,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DatedLogFile {
    path: PathBuf,
    date: Date,
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use pretty_assertions::assert_eq;

    use super::{LogFileProtection, RetentionCleaner, start_retention_worker_with};
    use crate::health::LoggingHealthHandle;
    use crate::{FileSystemAction, LoggingHealthStatus, LoggingInitError, LoggingIssue};

    /// Blocks cleanup until the test proves scheduling did not block the caller.
    struct GatedCleaner {
        started: mpsc::Sender<PathBuf>,
        resume: mpsc::Receiver<()>,
    }

    impl RetentionCleaner for GatedCleaner {
        fn cleanup(&self, protection: &LogFileProtection) -> Result<(), LoggingInitError> {
            self.started.send(protection.current_log_path()).unwrap();
            self.resume.recv().unwrap();
            Ok(())
        }
    }

    /// Verifies a blocked cleanup remains isolated from subsequent scheduling calls.
    #[test]
    fn schedules_cleanup_without_waiting_for_the_retention_worker() {
        let health = LoggingHealthHandle::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let protection = LogFileProtection::new(PathBuf::from("logs/ora.log.2026-07-01"));
        let runtime = start_retention_worker_with(
            GatedCleaner {
                started: started_tx,
                resume: resume_rx,
            },
            PathBuf::from("logs"),
            protection,
            health.recorder(),
        );

        runtime
            .handle
            .protect_rotation_target(PathBuf::from("logs/ora.log.2026-07-02"));
        runtime
            .handle
            .activate(PathBuf::from("logs/ora.log.2026-07-02"));
        runtime.handle.schedule();
        assert_eq!(
            started_rx
                .recv_timeout(Duration::from_secs(/*secs*/ 5))
                .unwrap(),
            PathBuf::from("logs/ora.log.2026-07-02")
        );

        // This call must only replace the latest path and enqueue one coalesced wake-up.
        runtime
            .handle
            .protect_rotation_target(PathBuf::from("logs/ora.log.2026-07-03"));
        runtime
            .handle
            .activate(PathBuf::from("logs/ora.log.2026-07-03"));
        runtime.handle.schedule();
        resume_tx.send(()).unwrap();
        drop(runtime);
    }

    /// Always reports a deterministic filesystem failure to the health recorder.
    struct FailingCleaner;

    impl RetentionCleaner for FailingCleaner {
        fn cleanup(&self, protection: &LogFileProtection) -> Result<(), LoggingInitError> {
            Err(LoggingInitError::FileSystem {
                action: FileSystemAction::RemoveFile,
                path: protection.current_log_path(),
                source: io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "simulated cleanup failure",
                ),
            })
        }
    }

    /// Verifies cleanup failures degrade health without being returned to the logging writer.
    #[test]
    fn reports_retention_failures_through_the_health_handle() {
        let health = LoggingHealthHandle::default();
        let protection = LogFileProtection::new(PathBuf::from("logs/ora.log.2026-07-01"));
        let runtime = start_retention_worker_with(
            FailingCleaner,
            PathBuf::from("logs"),
            protection,
            health.recorder(),
        );
        runtime
            .handle
            .protect_rotation_target(PathBuf::from("logs/ora.log.2026-07-02"));
        runtime
            .handle
            .activate(PathBuf::from("logs/ora.log.2026-07-02"));
        runtime.handle.schedule();

        let deadline = std::time::Instant::now() + Duration::from_secs(/*secs*/ 5);
        let snapshot = loop {
            let snapshot = health.snapshot();
            if snapshot.counters.retention_failures > 0 {
                break snapshot;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        };
        drop(runtime);

        assert_eq!(
            snapshot.status,
            LoggingHealthStatus::Degraded {
                primary: LoggingIssue::RetentionFailed {
                    directory: PathBuf::from("logs"),
                    error:
                        "failed to RemoveFile at logs/ora.log.2026-07-02: simulated cleanup failure"
                            .to_string(),
                },
                additional: Vec::new(),
            }
        );
    }
}
