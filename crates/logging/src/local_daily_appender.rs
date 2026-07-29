use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::path::Path;

use time::{Date, OffsetDateTime};

use crate::clock::local_date_at;
use crate::file_output::{ActiveLogPath, cleanup_old_logs};
use crate::{FileSystemAction, LoggingInitError};

/// Supplies absolute instants to local-calendar rotation without coupling tests to wall time.
pub(crate) trait TimeSource: Send + 'static {
    /// Returns the current absolute instant used to select the active local date.
    fn now(&self) -> OffsetDateTime;
}

/// Opens append-only log files so rotation failures can be exercised without platform tricks.
pub(crate) trait FileOpener: Send + 'static {
    /// Opens or creates the file that owns one local calendar day's events.
    fn open(&self, path: &Path) -> io::Result<File>;
}

/// Reads wall time from the operating system for production file outputs.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemTimeSource;

impl TimeSource for SystemTimeSource {
    fn now(&self) -> OffsetDateTime {
        // UTC is only the unambiguous source instant; the appender applies its configured IANA
        // timezone before this value can select a filename or rollover boundary.
        OffsetDateTime::now_utc()
    }
}

/// Uses standard append semantics for production log files.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemFileOpener;

impl FileOpener for SystemFileOpener {
    fn open(&self, path: &Path) -> io::Result<File> {
        OpenOptions::new().create(true).append(true).open(path)
    }
}

/// Writes each configured local calendar date into its matching append-only file.
pub(crate) struct LocalDailyAppender<T, O> {
    active_path: ActiveLogPath,
    timezone: chrono_tz::Tz,
    max_days: NonZeroUsize,
    active_date: Date,
    active_file: File,
    time_source: T,
    file_opener: O,
}

impl<T, O> LocalDailyAppender<T, O>
where
    T: TimeSource,
    O: FileOpener,
{
    /// Opens the current local-date file and enforces retention before accepting events.
    pub(crate) fn new(
        active_path: ActiveLogPath,
        timezone: chrono_tz::Tz,
        max_days: NonZeroUsize,
        time_source: T,
        file_opener: O,
    ) -> Result<Self, LoggingInitError> {
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
        cleanup_old_logs(&active_path, max_days, &active_file_path)?;

        Ok(Self {
            active_path,
            timezone,
            max_days,
            active_date,
            active_file,
            time_source,
            file_opener,
        })
    }

    /// Advances to a later local-date file while keeping the old file usable after open failures.
    fn rotate_if_needed(&mut self) {
        let current_date = local_date_at(self.time_source.now(), self.timezone);
        if current_date <= self.active_date {
            return;
        }

        let next_file_path = self.active_path.path_for_date(current_date);
        // This sink cannot emit a structured event about itself without recursively writing it.
        let next_file = match self.file_opener.open(&next_file_path) {
            Ok(next_file) => next_file,
            Err(error) => {
                eprintln!(
                    "failed to open rotated log file at {}: {error}",
                    next_file_path.display()
                );
                return;
            }
        };

        if let Err(error) = self.active_file.flush() {
            eprintln!("failed to flush the previous daily log file: {error}");
        }
        self.active_file = next_file;
        self.active_date = current_date;

        // Runtime retention cannot interrupt a healthy new sink after startup has completed.
        if let Err(error) = cleanup_old_logs(&self.active_path, self.max_days, &next_file_path) {
            eprintln!("failed to clean up rotated log files: {error}");
        }
    }
}

impl<T, O> Write for LocalDailyAppender<T, O>
where
    T: TimeSource,
    O: FileOpener,
{
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.rotate_if_needed();
        self.active_file.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.active_file.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{self, Write};
    use std::num::NonZeroUsize;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use time::{OffsetDateTime, macros::datetime};

    use super::{FileOpener, LocalDailyAppender, SystemFileOpener, TimeSource};
    use crate::file_output::ActiveLogPath;
    use crate::{FileSystemAction, LoggingInitError};

    /// Allows each test to move wall time across calendar boundaries without waiting.
    #[derive(Clone, Debug)]
    struct TestTimeSource {
        now: Arc<Mutex<OffsetDateTime>>,
    }

    impl TestTimeSource {
        /// Creates a controllable clock fixed at one initial instant.
        fn new(now: OffsetDateTime) -> Self {
            Self {
                now: Arc::new(Mutex::new(now)),
            }
        }

        /// Moves the test clock to another absolute instant.
        fn set(&self, now: OffsetDateTime) {
            *self.now.lock().unwrap() = now;
        }
    }

    impl TimeSource for TestTimeSource {
        fn now(&self) -> OffsetDateTime {
            *self.now.lock().unwrap()
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

        assert_eq!(
            read_file_names(&temp_dir),
            vec![
                "ora.log.2026-07-03".to_string(),
                "ora.log.2026-07-04".to_string(),
                "ora.log.2026-07-05".to_string(),
            ]
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
        appender.write_all(b"after retry\n").unwrap();
        appender.flush().unwrap();

        assert_eq!(
            read_log_files(&temp_dir),
            vec![
                (
                    "ora.log.2026-07-01".to_string(),
                    "before\nduring failure\n".to_string(),
                ),
                (
                    "ora.log.2026-07-02".to_string(),
                    "after retry\n".to_string(),
                ),
            ]
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

        let error = match LocalDailyAppender::new(
            active_path,
            chrono_tz::UTC,
            NonZeroUsize::new(/*n*/ 3).unwrap(),
            TestTimeSource::new(datetime!(2026-07-01 12:00 UTC)),
            file_opener,
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
    ) -> LocalDailyAppender<TestTimeSource, ControlledFileOpener> {
        LocalDailyAppender::new(
            ActiveLogPath::from_path(&temp_dir.path().join("ora.log")).unwrap(),
            timezone,
            NonZeroUsize::new(max_days).unwrap(),
            time_source,
            file_opener,
        )
        .unwrap()
    }

    /// Reads sorted file names so boundary and retention tests compare complete outcomes.
    fn read_file_names(temp_dir: &TempDir) -> Vec<String> {
        read_log_files(temp_dir)
            .into_iter()
            .map(|(file_name, _)| file_name)
            .collect()
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
