use std::fs::File;
use std::io::{self, Write};

use time::Date;

use crate::appender_dependencies::{FileOpener, TimeSource};
use crate::clock::local_date_at;
use crate::file_output::ActiveLogPath;
use crate::health::LoggingHealthRecorder;
use crate::retention::{
    LogFileProtection, RetentionCleaner, RetentionHandle, RetentionWorkerGuard,
    RotationTargetProtection, start_retention_worker_with,
};
use crate::rotation_retry::RotationRetry;
use crate::{FileSystemAction, LoggingInitError};

/// Routes each write to the append-only file for the worker's current local processing date.
pub(crate) struct LocalDailyAppender<T, O> {
    active_path: ActiveLogPath,
    timezone: chrono_tz::Tz,
    active_date: Date,
    active_file: File,
    time_source: T,
    file_opener: O,
    rotation_retry: RotationRetry,
    retention: RetentionHandle,
    health: LoggingHealthRecorder,
}

/// Couples the appender with the independent worker that owns runtime retention.
pub(crate) struct LocalDailyAppenderRuntime<T, O> {
    pub(crate) appender: LocalDailyAppender<T, O>,
    pub(crate) retention_guard: RetentionWorkerGuard,
}

impl<T, O> LocalDailyAppender<T, O>
where
    T: TimeSource,
    O: FileOpener,
{
    /// Opens the current file, records non-fatal startup cleanup failures, and starts retention.
    pub(crate) fn prepare<C>(
        active_path: ActiveLogPath,
        timezone: chrono_tz::Tz,
        time_source: T,
        file_opener: O,
        cleaner: C,
        health: LoggingHealthRecorder,
    ) -> Result<LocalDailyAppenderRuntime<T, O>, LoggingInitError>
    where
        C: RetentionCleaner,
    {
        let active_date = local_date_at(time_source.now(), timezone);
        let active_file_path = active_path.path_for_date(active_date);
        let active_file =
            file_opener
                .open(&active_file_path)
                .map_err(|source| LoggingInitError::FileSystem {
                    action: FileSystemAction::OpenFile,
                    path: active_file_path.clone(),
                    source,
                })?;
        let protection = LogFileProtection::new(active_file_path);
        if let Err(error) = cleaner.cleanup(&protection) {
            health.record_retention_failure(active_path.directory().to_path_buf(), &error);
        }
        let retention_runtime = start_retention_worker_with(
            cleaner,
            active_path.directory().to_path_buf(),
            protection,
            health.clone(),
        );

        Ok(LocalDailyAppenderRuntime {
            appender: Self {
                active_path,
                timezone,
                active_date,
                active_file,
                time_source,
                file_opener,
                rotation_retry: RotationRetry::Ready,
                retention: retention_runtime.handle,
                health,
            },
            retention_guard: retention_runtime.guard,
        })
    }

    /// Advances to a later local-date file while keeping the old file usable after open failures.
    fn rotate_if_needed(&mut self) {
        let current_date = local_date_at(self.time_source.now(), self.timezone);
        if current_date <= self.active_date {
            return;
        }
        let monotonic_elapsed = self.time_source.monotonic_elapsed();
        if !self
            .rotation_retry
            .allows_attempt(current_date, monotonic_elapsed)
        {
            return;
        }

        let next_file_path = self.active_path.path_for_date(current_date);
        match self
            .retention
            .protect_rotation_target(next_file_path.clone())
        {
            RotationTargetProtection::Protected => {}
            RotationTargetProtection::DeletionInProgress => {
                let error = io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "retention cleanup is deleting the rollover target",
                );
                self.health.record_rotation_failure(next_file_path, &error);
                self.rotation_retry
                    .record_failure(current_date, monotonic_elapsed);
                return;
            }
        }
        let next_file = match self.file_opener.open(&next_file_path) {
            Ok(next_file) => next_file,
            Err(error) => {
                self.health.record_rotation_failure(next_file_path, &error);
                self.rotation_retry
                    .record_failure(current_date, monotonic_elapsed);
                return;
            }
        };

        if let Err(error) = self.active_file.flush() {
            self.health.record_output_flush_failure(
                self.active_path.path_for_date(self.active_date),
                &error,
            );
        }
        self.active_file = next_file;
        self.active_date = current_date;
        self.retention.activate(next_file_path);
        self.rotation_retry = RotationRetry::Ready;
        self.health.record_rotation_recovered();
        self.retention.schedule();
    }
}

impl<T, O> Write for LocalDailyAppender<T, O>
where
    T: TimeSource,
    O: FileOpener,
{
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.rotate_if_needed();
        let result = self.active_file.write(buffer);
        match &result {
            Ok(_) => self.health.record_output_write_recovered(),
            Err(error) => self.health.record_output_write_failure(
                self.active_path.path_for_date(self.active_date),
                error,
            ),
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        let result = self.active_file.flush();
        match &result {
            Ok(()) => self.health.record_output_flush_recovered(),
            Err(error) => self.health.record_output_flush_failure(
                self.active_path.path_for_date(self.active_date),
                error,
            ),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{self, Write};
    use std::num::NonZeroUsize;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use time::{OffsetDateTime, macros::datetime};

    use super::LocalDailyAppender;
    use crate::appender_dependencies::{FileOpener, SystemFileOpener, TimeSource};
    use crate::file_output::ActiveLogPath;
    use crate::health::LoggingHealthHandle;
    use crate::retention::{
        FilesystemRetentionCleaner, LogFileProtection, RetentionCleaner, RetentionWorkerGuard,
    };
    use crate::{
        FileSystemAction, LoggingHealthSnapshot, LoggingHealthStatus, LoggingInitError,
        LoggingIssue,
    };

    /// Allows each test to move wall time across calendar boundaries without waiting.
    #[derive(Clone, Debug)]
    struct TestTimeSource {
        now: Arc<Mutex<OffsetDateTime>>,
        monotonic_elapsed: Arc<Mutex<Duration>>,
    }

    impl TestTimeSource {
        /// Creates a controllable clock fixed at one initial instant.
        fn new(now: OffsetDateTime) -> Self {
            Self {
                now: Arc::new(Mutex::new(now)),
                monotonic_elapsed: Arc::new(Mutex::new(Duration::ZERO)),
            }
        }

        /// Moves the test clock to another absolute instant.
        fn set(&self, now: OffsetDateTime) {
            *self.now.lock().unwrap() = now;
        }

        /// Advances only monotonic time so retry backoff tests do not sleep.
        fn advance(&self, duration: Duration) {
            let mut elapsed = self.monotonic_elapsed.lock().unwrap();
            *elapsed = elapsed.saturating_add(duration);
        }
    }

    impl TimeSource for TestTimeSource {
        fn now(&self) -> OffsetDateTime {
            *self.now.lock().unwrap()
        }

        fn monotonic_elapsed(&self) -> Duration {
            *self.monotonic_elapsed.lock().unwrap()
        }
    }

    /// Pauses the first worker-time read after initialization so queue timing is deterministic.
    #[derive(Debug)]
    struct GatedWorkerTimeSource {
        initial_now: OffsetDateTime,
        initialization_read: AtomicBool,
        worker_waiting: mpsc::Sender<()>,
        resumed_now: mpsc::Receiver<OffsetDateTime>,
    }

    impl TimeSource for GatedWorkerTimeSource {
        fn now(&self) -> OffsetDateTime {
            if !self.initialization_read.swap(true, Ordering::SeqCst) {
                return self.initial_now;
            }

            self.worker_waiting.send(()).unwrap();
            self.resumed_now.recv().unwrap()
        }

        fn monotonic_elapsed(&self) -> Duration {
            Duration::ZERO
        }
    }

    /// Delegates normal opens to the filesystem while permitting one path to fail deterministically.
    #[derive(Clone, Debug, Default)]
    struct ControlledFileOpener {
        rejected_path: Arc<Mutex<Option<PathBuf>>>,
    }

    impl ControlledFileOpener {
        /// Rejects subsequent attempts to open the selected path.
        fn reject(&self, path: PathBuf) {
            *self.rejected_path.lock().unwrap() = Some(path);
        }

        /// Restores normal file opening after a simulated failure.
        fn allow_all(&self) {
            *self.rejected_path.lock().unwrap() = None;
        }
    }

    impl FileOpener for ControlledFileOpener {
        fn open(&self, path: &Path) -> io::Result<std::fs::File> {
            if self.rejected_path.lock().unwrap().as_deref() == Some(path) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "simulated open failure",
                ));
            }

            SystemFileOpener.open(path)
        }
    }

    /// Keeps the retention worker alive while exposing the appender through the Write contract.
    struct TestAppender {
        appender: LocalDailyAppender<TestTimeSource, ControlledFileOpener>,
        _retention_guard: RetentionWorkerGuard,
        health: LoggingHealthHandle,
    }

    impl TestAppender {
        /// Returns the complete backend-visible health state for assertions.
        fn health(&self) -> LoggingHealthSnapshot {
            self.health.snapshot()
        }
    }

    impl Write for TestAppender {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.appender.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.appender.flush()
        }
    }

    /// Verifies Shanghai rotates when UTC reaches the previous day's 16:00 boundary.
    #[test]
    fn rotates_at_shanghai_midnight() {
        assert_rotation_boundary(
            chrono_tz::Asia::Shanghai,
            datetime!(2026-07-29 15:59:59 UTC),
            datetime!(2026-07-29 16:00 UTC),
            ["ora.log.2026-07-29", "ora.log.2026-07-30"],
        );
    }

    /// Verifies London applies the winter offset when selecting the daily boundary.
    #[test]
    fn rotates_at_london_winter_midnight() {
        assert_rotation_boundary(
            chrono_tz::Europe::London,
            datetime!(2026-01-14 23:59:59 UTC),
            datetime!(2026-01-15 00:00 UTC),
            ["ora.log.2026-01-14", "ora.log.2026-01-15"],
        );
    }

    /// Verifies London applies the summer offset when selecting the daily boundary.
    #[test]
    fn rotates_at_london_summer_midnight() {
        assert_rotation_boundary(
            chrono_tz::Europe::London,
            datetime!(2026-07-14 22:59:59 UTC),
            datetime!(2026-07-14 23:00 UTC),
            ["ora.log.2026-07-14", "ora.log.2026-07-15"],
        );
    }

    /// Verifies UTC configurations retain the existing UTC-midnight behavior.
    #[test]
    fn rotates_at_utc_midnight() {
        assert_rotation_boundary(
            chrono_tz::UTC,
            datetime!(2026-07-29 23:59:59 UTC),
            datetime!(2026-07-30 00:00 UTC),
            ["ora.log.2026-07-29", "ora.log.2026-07-30"],
        );
    }

    /// Verifies queued bytes use the worker's processing date without changing their timestamp.
    #[test]
    fn queued_event_after_midnight_uses_worker_processing_date() {
        const EVENT: &[u8] = concat!(
            r#"{"timestamp":"2026-07-29T23:59:59+08:00","#,
            r#""message":"queued before midnight"}"#,
            "\n",
        )
        .as_bytes();

        let temp_dir = TempDir::new().unwrap();
        let (worker_waiting_tx, worker_waiting_rx) = mpsc::channel();
        let (resume_worker_tx, resume_worker_rx) = mpsc::channel();
        let active_path = ActiveLogPath::from_path(&temp_dir.path().join("ora.log")).unwrap();
        let health = LoggingHealthHandle::default();
        let runtime = LocalDailyAppender::prepare(
            active_path.clone(),
            chrono_tz::Asia::Shanghai,
            GatedWorkerTimeSource {
                initial_now: datetime!(2026-07-29 15:59:59 UTC),
                initialization_read: AtomicBool::new(false),
                worker_waiting: worker_waiting_tx,
                resumed_now: resume_worker_rx,
            },
            ControlledFileOpener::default(),
            FilesystemRetentionCleaner::new(active_path, NonZeroUsize::new(/*n*/ 3).unwrap()),
            health.recorder(),
        )
        .unwrap();
        let (mut writer, writer_guard) = tracing_appender::non_blocking(runtime.appender);

        writer.write_all(EVENT).unwrap();
        worker_waiting_rx
            .recv_timeout(Duration::from_secs(/*secs*/ 5))
            .unwrap();
        resume_worker_tx
            .send(datetime!(2026-07-29 16:00 UTC))
            .unwrap();
        drop(writer);
        drop(writer_guard);
        drop(runtime.retention_guard);

        assert_eq!(
            read_log_files(&temp_dir),
            vec![
                ("ora.log.2026-07-29".to_string(), String::new()),
                (
                    "ora.log.2026-07-30".to_string(),
                    String::from_utf8(EVENT.to_vec()).unwrap(),
                ),
            ]
        );
    }

    /// Verifies a cross-date clock rollback cannot reopen an older daily file.
    #[test]
    fn does_not_rotate_backwards_after_clock_rollback() {
        let temp_dir = TempDir::new().unwrap();
        let time_source = TestTimeSource::new(datetime!(2026-07-30 00:01 UTC));
        let mut appender = new_test_appender(
            &temp_dir,
            chrono_tz::UTC,
            /*max_days*/ 3,
            time_source.clone(),
            ControlledFileOpener::default(),
        );
        appender.write_all(b"newer\n").unwrap();

        time_source.set(datetime!(2026-07-29 23:59 UTC));
        appender.write_all(b"after rollback\n").unwrap();
        appender.flush().unwrap();

        assert_eq!(
            read_log_files(&temp_dir),
            vec![(
                "ora.log.2026-07-30".to_string(),
                "newer\nafter rollback\n".to_string(),
            )]
        );
    }

    /// Verifies every successful runtime rotation reapplies the configured retention limit.
    #[test]
    fn enforces_retention_across_runtime_rotations() {
        let temp_dir = TempDir::new().unwrap();
        let time_source = TestTimeSource::new(datetime!(2026-07-01 12:00 UTC));
        let mut appender = new_test_appender(
            &temp_dir,
            chrono_tz::UTC,
            /*max_days*/ 3,
            time_source.clone(),
            ControlledFileOpener::default(),
        );

        for day in 1..=5 {
            time_source.set(
                OffsetDateTime::from_unix_timestamp(
                    datetime!(2026-07-01 12:00 UTC).unix_timestamp() + i64::from(day - 1) * 86_400,
                )
                .unwrap(),
            );
            appender
                .write_all(format!("day {day}\n").as_bytes())
                .unwrap();
        }
        appender.flush().unwrap();

        let expected_file_names = vec![
            "ora.log.2026-07-03".to_string(),
            "ora.log.2026-07-04".to_string(),
            "ora.log.2026-07-05".to_string(),
        ];
        wait_for_file_names(&temp_dir, &expected_file_names);
        assert_eq!(read_file_names(&temp_dir), expected_file_names);
    }

    /// Verifies coalesced cleanup never removes the newest file during rapid date advances.
    #[test]
    fn preserves_the_active_file_during_rapid_rotations() {
        let temp_dir = TempDir::new().unwrap();
        let time_source = TestTimeSource::new(datetime!(2026-07-01 12:00 UTC));
        let mut appender = new_test_appender(
            &temp_dir,
            chrono_tz::UTC,
            /*max_days*/ 1,
            time_source.clone(),
            ControlledFileOpener::default(),
        );

        for day in 1..=5 {
            time_source.set(
                OffsetDateTime::from_unix_timestamp(
                    datetime!(2026-07-01 12:00 UTC).unix_timestamp() + i64::from(day - 1) * 86_400,
                )
                .unwrap(),
            );
            appender
                .write_all(format!("day {day}\n").as_bytes())
                .unwrap();
        }
        appender.flush().unwrap();

        let expected_file_names = vec!["ora.log.2026-07-05".to_string()];
        wait_for_file_names(&temp_dir, &expected_file_names);
        assert_eq!(
            read_log_files(&temp_dir),
            vec![("ora.log.2026-07-05".to_string(), "day 5\n".to_string(),)]
        );
    }

    /// Verifies a failed rotation preserves the event and retries the new file on a later write.
    #[test]
    fn keeps_writing_the_old_file_until_rotation_open_succeeds() {
        let temp_dir = TempDir::new().unwrap();
        let time_source = TestTimeSource::new(datetime!(2026-07-01 12:00 UTC));
        let file_opener = ControlledFileOpener::default();
        let mut appender = new_test_appender(
            &temp_dir,
            chrono_tz::UTC,
            /*max_days*/ 3,
            time_source.clone(),
            file_opener.clone(),
        );
        appender.write_all(b"before\n").unwrap();

        let next_file_path = temp_dir.path().join("ora.log.2026-07-02");
        file_opener.reject(next_file_path);
        time_source.set(datetime!(2026-07-02 00:00 UTC));
        appender.write_all(b"during failure\n").unwrap();

        file_opener.allow_all();
        appender.write_all(b"before retry delay\n").unwrap();
        time_source.advance(Duration::from_secs(/*secs*/ 1));
        appender.write_all(b"after retry\n").unwrap();
        appender.flush().unwrap();

        assert_eq!(
            read_log_files(&temp_dir),
            vec![
                (
                    "ora.log.2026-07-01".to_string(),
                    "before\nduring failure\nbefore retry delay\n".to_string(),
                ),
                (
                    "ora.log.2026-07-02".to_string(),
                    "after retry\n".to_string(),
                ),
            ]
        );
        assert_eq!(
            appender.health(),
            LoggingHealthSnapshot {
                status: LoggingHealthStatus::Healthy,
                counters: crate::LoggingHealthCounters {
                    rotation_open_failures: 1,
                    ..crate::LoggingHealthCounters::default()
                },
            }
        );
    }

    /// Verifies the first file-open failure is returned as typed initialization context.
    #[test]
    fn reports_initial_file_open_failures() {
        let temp_dir = TempDir::new().unwrap();
        let active_path = ActiveLogPath::from_path(&temp_dir.path().join("ora.log")).unwrap();
        let expected_path = temp_dir.path().join("ora.log.2026-07-01");
        let file_opener = ControlledFileOpener::default();
        file_opener.reject(expected_path.clone());

        let health = LoggingHealthHandle::default();
        let error = match LocalDailyAppender::prepare(
            active_path.clone(),
            chrono_tz::UTC,
            TestTimeSource::new(datetime!(2026-07-01 12:00 UTC)),
            file_opener,
            FilesystemRetentionCleaner::new(active_path, NonZeroUsize::new(/*n*/ 3).unwrap()),
            health.recorder(),
        ) {
            Ok(_) => panic!("the simulated open failure must reject initialization"),
            Err(error) => error,
        };

        assert_eq!(
            match error {
                LoggingInitError::FileSystem {
                    action,
                    path,
                    source,
                } => Some((action, path, source.kind())),
                _ => None,
            },
            Some((
                FileSystemAction::OpenFile,
                expected_path,
                io::ErrorKind::PermissionDenied,
            ))
        );
    }

    /// Rejects every retention pass while allowing the active log file to remain usable.
    struct FailingRetentionCleaner;

    impl RetentionCleaner for FailingRetentionCleaner {
        fn cleanup(&self, protection: &LogFileProtection) -> Result<(), LoggingInitError> {
            Err(LoggingInitError::FileSystem {
                action: FileSystemAction::RemoveFile,
                path: protection.current_log_path(),
                source: io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "simulated startup cleanup failure",
                ),
            })
        }
    }

    /// Verifies old-log deletion failures degrade health but do not reject initialization.
    #[test]
    fn allows_startup_when_initial_retention_cleanup_fails() {
        let temp_dir = TempDir::new().unwrap();
        let active_path = ActiveLogPath::from_path(&temp_dir.path().join("ora.log")).unwrap();
        let active_file_path = temp_dir.path().join("ora.log.2026-07-01");
        let health = LoggingHealthHandle::default();

        let runtime = LocalDailyAppender::prepare(
            active_path,
            chrono_tz::UTC,
            TestTimeSource::new(datetime!(2026-07-01 12:00 UTC)),
            ControlledFileOpener::default(),
            FailingRetentionCleaner,
            health.recorder(),
        )
        .unwrap();

        assert_eq!(
            health.snapshot(),
            LoggingHealthSnapshot {
                status: LoggingHealthStatus::Degraded {
                    primary: LoggingIssue::RetentionFailed {
                        directory: temp_dir.path().to_path_buf(),
                        error: format!(
                            "failed to RemoveFile at {}: simulated startup cleanup failure",
                            active_file_path.display()
                        ),
                    },
                    additional: Vec::new(),
                },
                counters: crate::LoggingHealthCounters {
                    retention_failures: 1,
                    ..crate::LoggingHealthCounters::default()
                },
            }
        );
        drop(runtime);
    }

    /// Exercises one before-and-after pair and compares the complete produced file set.
    fn assert_rotation_boundary(
        timezone: chrono_tz::Tz,
        before_midnight: OffsetDateTime,
        at_midnight: OffsetDateTime,
        expected_file_names: [&str; 2],
    ) {
        let temp_dir = TempDir::new().unwrap();
        let time_source = TestTimeSource::new(before_midnight);
        let mut appender = new_test_appender(
            &temp_dir,
            timezone,
            /*max_days*/ 3,
            time_source.clone(),
            ControlledFileOpener::default(),
        );
        appender.write_all(b"before\n").unwrap();

        time_source.set(at_midnight);
        appender.write_all(b"after\n").unwrap();
        appender.flush().unwrap();

        let [before_file_name, after_file_name] = expected_file_names;
        assert_eq!(
            read_log_files(&temp_dir),
            vec![
                (before_file_name.to_string(), "before\n".to_string()),
                (after_file_name.to_string(), "after\n".to_string()),
            ]
        );
    }

    /// Builds a test appender with an explicit clock and opener.
    fn new_test_appender(
        temp_dir: &TempDir,
        timezone: chrono_tz::Tz,
        max_days: usize,
        time_source: TestTimeSource,
        file_opener: ControlledFileOpener,
    ) -> TestAppender {
        let active_path = ActiveLogPath::from_path(&temp_dir.path().join("ora.log")).unwrap();
        let health = LoggingHealthHandle::default();
        let runtime = LocalDailyAppender::prepare(
            active_path.clone(),
            timezone,
            time_source,
            file_opener,
            FilesystemRetentionCleaner::new(active_path, NonZeroUsize::new(max_days).unwrap()),
            health.recorder(),
        )
        .unwrap();

        TestAppender {
            appender: runtime.appender,
            _retention_guard: runtime.retention_guard,
            health,
        }
    }

    /// Reads sorted file names so boundary and retention tests compare complete outcomes.
    fn read_file_names(temp_dir: &TempDir) -> Vec<String> {
        let mut file_names = fs::read_dir(temp_dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        file_names.sort();
        file_names
    }

    /// Waits for asynchronous retention to produce the expected complete file set.
    fn wait_for_file_names(temp_dir: &TempDir, expected: &[String]) {
        let deadline = Instant::now() + Duration::from_secs(/*secs*/ 5);
        loop {
            let actual = read_file_names(temp_dir);
            if actual == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "retention did not converge: expected {expected:?}, actual {actual:?}"
            );
            std::thread::sleep(Duration::from_millis(/*millis*/ 10));
        }
    }

    /// Reads every sorted file and its full contents from one temporary log directory.
    fn read_log_files(temp_dir: &TempDir) -> Vec<(String, String)> {
        let mut files = fs::read_dir(temp_dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .map(|path| {
                (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    fs::read_to_string(path).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }
}
