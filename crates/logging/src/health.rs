use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tracing_appender::non_blocking::ErrorCounter;

const ROTATION_ISSUE: u8 = 1 << 0;
const OUTPUT_WRITE_ISSUE: u8 = 1 << 1;
const OUTPUT_FLUSH_ISSUE: u8 = 1 << 2;
const RETENTION_ISSUE: u8 = 1 << 3;

/// Provides a cloneable, passive view of logging failures without emitting recursive log events.
#[derive(Clone, Debug, Default)]
pub struct LoggingHealthHandle {
    shared: Arc<LoggingHealthShared>,
}

impl LoggingHealthHandle {
    /// Returns the current degradation state together with cumulative failure counters.
    pub fn snapshot(&self) -> LoggingHealthSnapshot {
        let dropped_events = self
            .shared
            .drop_counters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .fold(/*init*/ 0usize, |count, counter| {
                count.saturating_add(counter.dropped_lines())
            });
        let active = self
            .shared
            .active_issues
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let counters = LoggingHealthCounters {
            rotation_open_failures: self.shared.rotation_open_failures.load(Ordering::Acquire),
            output_write_failures: self.shared.output_write_failures.load(Ordering::Acquire),
            output_flush_failures: self.shared.output_flush_failures.load(Ordering::Acquire),
            retention_failures: self.shared.retention_failures.load(Ordering::Acquire),
            dropped_events,
        };
        let dropped_issue = (dropped_events > 0).then_some(LoggingIssue::EventsDropped {
            count: dropped_events,
        });
        let mut issues = [
            active.rotation.clone(),
            active.output_write.clone(),
            active.output_flush.clone(),
            active.retention.clone(),
        ]
        .into_iter()
        .flatten()
        .chain(dropped_issue);
        let status = match issues.next() {
            Some(primary) => LoggingHealthStatus::Degraded {
                primary,
                additional: issues.collect(),
            },
            None => LoggingHealthStatus::Healthy,
        };

        LoggingHealthSnapshot { status, counters }
    }

    /// Creates the internal recorder shared by file writers and retention workers.
    pub(crate) fn recorder(&self) -> LoggingHealthRecorder {
        LoggingHealthRecorder {
            shared: self.shared.clone(),
        }
    }

    /// Adds one lossy non-blocking writer counter to the aggregated health snapshot.
    pub(crate) fn add_drop_counter(&self, counter: ErrorCounter) {
        self.shared
            .drop_counters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(counter);
    }
}

/// Captures both the current logging status and counters that remain useful after recovery.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LoggingHealthSnapshot {
    pub status: LoggingHealthStatus,
    pub counters: LoggingHealthCounters,
}

/// Distinguishes a healthy logger from one or more active degradation conditions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LoggingHealthStatus {
    Healthy,
    Degraded {
        primary: LoggingIssue,
        additional: Vec<LoggingIssue>,
    },
}

/// Describes one active condition that can reduce logging reliability or retention.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoggingIssue {
    RotationRetrying { path: PathBuf, error: String },
    OutputWriteFailed { path: PathBuf, error: String },
    OutputFlushFailed { path: PathBuf, error: String },
    RetentionFailed { directory: PathBuf, error: String },
    EventsDropped { count: usize },
}

/// Counts logging failures even after the corresponding active issue recovers.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct LoggingHealthCounters {
    pub rotation_open_failures: usize,
    pub output_write_failures: usize,
    pub output_flush_failures: usize,
    pub retention_failures: usize,
    pub dropped_events: usize,
}

/// Records failures from logging-owned threads without depending on the tracing subscriber.
#[derive(Clone, Debug)]
pub(crate) struct LoggingHealthRecorder {
    shared: Arc<LoggingHealthShared>,
}

impl LoggingHealthRecorder {
    /// Marks daily rotation as retrying after the next dated file cannot be opened.
    pub(crate) fn record_rotation_failure(&self, path: PathBuf, error: &std::io::Error) {
        self.set_issue(
            ROTATION_ISSUE,
            IssueSlot::Rotation,
            LoggingIssue::RotationRetrying {
                path,
                error: error.to_string(),
            },
            &self.shared.rotation_open_failures,
        );
    }

    /// Clears the active rotation issue after a later attempt opens the target file.
    pub(crate) fn record_rotation_recovered(&self) {
        self.clear_issue(ROTATION_ISSUE, IssueSlot::Rotation);
    }

    /// Marks the active file as degraded after a normal log write fails.
    pub(crate) fn record_output_write_failure(&self, path: PathBuf, error: &std::io::Error) {
        self.set_issue(
            OUTPUT_WRITE_ISSUE,
            IssueSlot::OutputWrite,
            LoggingIssue::OutputWriteFailed {
                path,
                error: error.to_string(),
            },
            &self.shared.output_write_failures,
        );
    }

    /// Clears the active write issue once the file accepts another write.
    pub(crate) fn record_output_write_recovered(&self) {
        self.clear_issue(OUTPUT_WRITE_ISSUE, IssueSlot::OutputWrite);
    }

    /// Marks the active file as degraded after a flush fails.
    pub(crate) fn record_output_flush_failure(&self, path: PathBuf, error: &std::io::Error) {
        self.set_issue(
            OUTPUT_FLUSH_ISSUE,
            IssueSlot::OutputFlush,
            LoggingIssue::OutputFlushFailed {
                path,
                error: error.to_string(),
            },
            &self.shared.output_flush_failures,
        );
    }

    /// Clears the active flush issue once a later flush succeeds.
    pub(crate) fn record_output_flush_recovered(&self) {
        self.clear_issue(OUTPUT_FLUSH_ISSUE, IssueSlot::OutputFlush);
    }

    /// Marks retention as degraded without preventing file logging or application startup.
    pub(crate) fn record_retention_failure(
        &self,
        directory: PathBuf,
        error: &(dyn Error + 'static),
    ) {
        self.record_retention_failure_message(directory, format_error_chain(error));
    }

    /// Marks retention as degraded when no typed source error is available.
    pub(crate) fn record_retention_failure_message(
        &self,
        directory: PathBuf,
        error: impl Into<String>,
    ) {
        self.set_issue(
            RETENTION_ISSUE,
            IssueSlot::Retention,
            LoggingIssue::RetentionFailed {
                directory,
                error: error.into(),
            },
            &self.shared.retention_failures,
        );
    }

    /// Clears the active retention issue after a complete cleanup succeeds.
    pub(crate) fn record_retention_recovered(&self) {
        self.clear_issue(RETENTION_ISSUE, IssueSlot::Retention);
    }

    /// Replaces one active issue and emits stderr only when entering that degraded state.
    fn set_issue(&self, flag: u8, slot: IssueSlot, issue: LoggingIssue, counter: &AtomicUsize) {
        let mut active = self
            .shared
            .active_issues
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slot.replace(&mut active, issue.clone());
        increment_saturating(counter);
        let was_active = self.shared.active_flags.fetch_or(flag, Ordering::AcqRel) & flag != 0;
        drop(active);

        // stderr remains a one-shot fallback for development shells; runtimes use snapshot().
        if !was_active {
            eprintln!("logging degraded: {issue:?}");
        }
    }

    /// Removes one recovered issue without locking on the common healthy success path.
    fn clear_issue(&self, flag: u8, slot: IssueSlot) {
        if self.shared.active_flags.load(Ordering::Acquire) & flag == 0 {
            return;
        }

        let mut active = self
            .shared
            .active_issues
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        slot.clear(&mut active);
        self.shared.active_flags.fetch_and(!flag, Ordering::AcqRel);
    }
}

#[derive(Debug, Default)]
struct LoggingHealthShared {
    active_flags: AtomicU8,
    active_issues: Mutex<ActiveIssues>,
    rotation_open_failures: AtomicUsize,
    output_write_failures: AtomicUsize,
    output_flush_failures: AtomicUsize,
    retention_failures: AtomicUsize,
    drop_counters: Mutex<Vec<ErrorCounter>>,
}

#[derive(Debug, Default)]
struct ActiveIssues {
    rotation: Option<LoggingIssue>,
    output_write: Option<LoggingIssue>,
    output_flush: Option<LoggingIssue>,
    retention: Option<LoggingIssue>,
}

#[derive(Clone, Copy, Debug)]
enum IssueSlot {
    Rotation,
    OutputWrite,
    OutputFlush,
    Retention,
}

impl IssueSlot {
    /// Replaces the issue owned by this failure category.
    fn replace(self, active: &mut ActiveIssues, issue: LoggingIssue) {
        match self {
            Self::Rotation => active.rotation = Some(issue),
            Self::OutputWrite => active.output_write = Some(issue),
            Self::OutputFlush => active.output_flush = Some(issue),
            Self::Retention => active.retention = Some(issue),
        }
    }

    /// Clears the issue owned by this failure category.
    fn clear(self, active: &mut ActiveIssues) {
        match self {
            Self::Rotation => active.rotation = None,
            Self::OutputWrite => active.output_write = None,
            Self::OutputFlush => active.output_flush = None,
            Self::Retention => active.retention = None,
        }
    }
}

/// Increments a cumulative counter without allowing wraparound to hide prior failures.
fn increment_saturating(counter: &AtomicUsize) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
        Some(value.saturating_add(1))
    });
}

/// Formats an error and all of its sources so passive health consumers receive actionable detail.
fn format_error_chain(error: &(dyn Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let _ = write!(&mut message, ": {cause}");
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tracing_appender::non_blocking::NonBlockingBuilder;

    use super::{
        LoggingHealthCounters, LoggingHealthHandle, LoggingHealthSnapshot, LoggingHealthStatus,
        LoggingIssue,
    };

    /// Pauses the worker's first write so the bounded queue can be filled deterministically.
    struct GatedWriter {
        block_next_write: bool,
        started: mpsc::Sender<()>,
        resume: mpsc::Receiver<()>,
    }

    impl Write for GatedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if self.block_next_write {
                self.block_next_write = false;
                self.started.send(()).unwrap();
                self.resume.recv().unwrap();
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Verifies the backend health API reports events discarded by the lossy queue.
    #[test]
    fn reports_lossy_queue_drops() {
        let health = LoggingHealthHandle::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let (mut writer, writer_guard) = NonBlockingBuilder::default()
            .buffered_lines_limit(/*buffered_lines_limit*/ 1)
            .lossy(/*is_lossy*/ true)
            .finish(GatedWriter {
                block_next_write: true,
                started: started_tx,
                resume: resume_rx,
            });
        health.add_drop_counter(writer.error_counter());

        writer.write_all(b"first\n").unwrap();
        started_rx
            .recv_timeout(Duration::from_secs(/*secs*/ 5))
            .unwrap();
        writer.write_all(b"second\n").unwrap();
        writer.write_all(b"dropped\n").unwrap();

        assert_eq!(
            health.snapshot(),
            LoggingHealthSnapshot {
                status: LoggingHealthStatus::Degraded {
                    primary: LoggingIssue::EventsDropped { count: 1 },
                    additional: Vec::new(),
                },
                counters: LoggingHealthCounters {
                    dropped_events: 1,
                    ..LoggingHealthCounters::default()
                },
            }
        );

        resume_tx.send(()).unwrap();
        drop(writer);
        drop(writer_guard);
    }

    /// Verifies active output issues recover while their cumulative counters remain available.
    #[test]
    fn preserves_failure_counters_after_output_recovers() {
        let health = LoggingHealthHandle::default();
        let recorder = health.recorder();
        let path = PathBuf::from("logs/ora.log.2026-07-31");
        recorder.record_output_write_failure(
            path.clone(),
            &io::Error::new(io::ErrorKind::StorageFull, "simulated full disk"),
        );
        recorder.record_output_flush_failure(
            path.clone(),
            &io::Error::new(io::ErrorKind::Other, "simulated flush failure"),
        );

        assert_eq!(
            health.snapshot(),
            LoggingHealthSnapshot {
                status: LoggingHealthStatus::Degraded {
                    primary: LoggingIssue::OutputWriteFailed {
                        path: path.clone(),
                        error: "simulated full disk".to_string(),
                    },
                    additional: vec![LoggingIssue::OutputFlushFailed {
                        path,
                        error: "simulated flush failure".to_string(),
                    }],
                },
                counters: LoggingHealthCounters {
                    output_write_failures: 1,
                    output_flush_failures: 1,
                    ..LoggingHealthCounters::default()
                },
            }
        );

        recorder.record_output_write_recovered();
        recorder.record_output_flush_recovered();

        assert_eq!(
            health.snapshot(),
            LoggingHealthSnapshot {
                status: LoggingHealthStatus::Healthy,
                counters: LoggingHealthCounters {
                    output_write_failures: 1,
                    output_flush_failures: 1,
                    ..LoggingHealthCounters::default()
                },
            }
        );
    }

    /// Verifies backend transports receive an explicitly tagged and stable JSON health shape.
    #[test]
    fn serializes_the_backend_health_contract() {
        let snapshot = LoggingHealthSnapshot {
            status: LoggingHealthStatus::Degraded {
                primary: LoggingIssue::EventsDropped { count: 2 },
                additional: Vec::new(),
            },
            counters: LoggingHealthCounters {
                dropped_events: 2,
                ..LoggingHealthCounters::default()
            },
        };

        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            json!({
                "status": {
                    "state": "degraded",
                    "primary": {
                        "kind": "events_dropped",
                        "count": 2,
                    },
                    "additional": [],
                },
                "counters": {
                    "rotation_open_failures": 0,
                    "output_write_failures": 0,
                    "output_flush_failures": 0,
                    "retention_failures": 0,
                    "dropped_events": 2,
                },
            })
        );
    }
}
