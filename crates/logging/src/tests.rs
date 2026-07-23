use std::fs;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use pretty_assertions::assert_eq;
use serde_json::Value;
use tempfile::TempDir;
use tracing::dispatcher::with_default;

use crate::file_output::{ActiveLogPath, cleanup_old_logs};
use crate::{
    FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, LoggingInitError, RotationPolicy,
    build_dispatch, init_logging, runtime_span, span_with_correlation,
};

/// Verifies the public configuration model stays small, explicit, and equality-friendly.
#[test]
fn preserves_the_public_configuration_model() {
    let config = LoggingConfig::new(
        LogLevel::Info,
        LogOutput::StdoutAndFile(FileLoggingConfig::new(
            "./ora.log",
            RotationPolicy::Daily,
            NonZeroUsize::new(3).unwrap(),
        )),
        chrono_tz::Asia::Shanghai,
    );

    assert_eq!(
        config,
        LoggingConfig {
            level: LogLevel::Info,
            output: LogOutput::StdoutAndFile(FileLoggingConfig {
                path: "./ora.log".into(),
                rotation: RotationPolicy::Daily,
                max_days: NonZeroUsize::new(3).unwrap(),
            }),
            timezone: chrono_tz::Asia::Shanghai,
        }
    );
}

/// Verifies the crate exports the intended API surface through public constructors and helpers.
#[test]
fn preserves_the_public_logging_api_surface() {
    let stdout_only = LoggingConfig::new(LogLevel::Debug, LogOutput::Stdout, chrono_tz::UTC);
    let warn_only = LoggingConfig::new(LogLevel::Warn, LogOutput::Stdout, chrono_tz::UTC);
    let file_only = FileLoggingConfig::new(
        "./ora.log",
        RotationPolicy::Daily,
        NonZeroUsize::new(5).unwrap(),
    );

    assert_eq!(stdout_only.level, LogLevel::Debug);
    assert_eq!(warn_only.level, LogLevel::Warn);
    assert_eq!(file_only.rotation, RotationPolicy::Daily);

    let stdout = SharedBuffer::default();
    let (dispatch, _guard) = build_dispatch(&stdout_only, stdout.make_writer()).unwrap();
    with_default(&dispatch, || {
        drop(runtime_span("bootstrap"));
    });
}

/// Verifies the warn level preserves warning and error events while filtering informational ones out.
#[test]
fn filters_events_at_warn_level() {
    let stdout = SharedBuffer::default();
    let (dispatch, _guard) = build_dispatch(
        &LoggingConfig::new(LogLevel::Warn, LogOutput::Stdout, chrono_tz::UTC),
        stdout.make_writer(),
    )
    .unwrap();

    with_default(&dispatch, || {
        tracing::info!(message = "ignored info");
        tracing::warn!(message = "kept warn");
        tracing::error!(message = "kept error");
    });

    let events = stdout.json_lines();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["level"], Value::String("WARN".to_string()));
    assert_eq!(events[0]["message"], Value::String("kept warn".to_string()));
    assert_eq!(events[1]["level"], Value::String("ERROR".to_string()));
    assert_eq!(
        events[1]["message"],
        Value::String("kept error".to_string())
    );
}

/// Verifies JSON output uses the shared top-level envelope plus nested context and error objects.
#[test]
fn formats_json_events_with_context_and_error_objects() {
    let temp_dir = TempDir::new().unwrap();
    let stdout = SharedBuffer::default();
    let file_config = FileLoggingConfig::new(
        temp_dir.path().join("ora.log"),
        RotationPolicy::Daily,
        NonZeroUsize::new(3).unwrap(),
    );
    let (dispatch, _guard) = build_dispatch(
        &LoggingConfig::new(
            LogLevel::Info,
            LogOutput::StdoutAndFile(file_config),
            chrono_tz::Asia::Shanghai,
        ),
        stdout.make_writer(),
    )
    .unwrap();

    with_default(&dispatch, || {
        let span = span_with_correlation("request", Some("trace-1"), Some("request-1"));
        let entered = span.enter();
        crate::ora_info!(
            message = "created project",
            operation = "create_project",
            project_id = "project-1",
            error.kind = "none"
        );
        drop(entered);
    });

    let stdout_events = stdout.json_lines();
    let stdout_event = &stdout_events[0];

    assert_eq!(stdout_event["level"], Value::String("INFO".to_string()));
    assert_eq!(
        stdout_event["target"],
        Value::String("ora_logging::tests".to_string())
    );
    assert_eq!(
        stdout_event["message"],
        Value::String("created project".to_string())
    );
    assert_eq!(
        stdout_event["method"],
        Value::String("formats_json_events_with_context_and_error_objects".to_string())
    );
    assert_eq!(stdout_event["span"], Value::String("request".to_string()));
    assert_eq!(
        stdout_event["trace_id"],
        Value::String("trace-1".to_string())
    );
    assert_eq!(
        stdout_event["request_id"],
        Value::String("request-1".to_string())
    );
    assert_eq!(
        stdout_event["context"],
        serde_json::json!({
            "operation": "create_project",
            "project_id": "project-1"
        })
    );
    assert_eq!(
        stdout_event["error"],
        serde_json::json!({
            "kind": "none"
        })
    );
    assert_eq!(
        stdout_event["timestamp"]
            .as_str()
            .map(|timestamp| timestamp.ends_with("+08:00")),
        Some(true)
    );
}

/// Verifies the wrapper macros add the current function name under the top-level `method` field.
#[test]
fn emits_method_field_from_wrapper_macros() {
    let stdout = SharedBuffer::default();
    let (dispatch, _guard) = build_dispatch(
        &LoggingConfig::new(LogLevel::Info, LogOutput::Stdout, chrono_tz::UTC),
        stdout.make_writer(),
    )
    .unwrap();

    with_default(&dispatch, || {
        crate::ora_info!(message = "macro event");
    });

    let event = &stdout.json_lines()[0];

    assert_eq!(
        event["method"],
        Value::String("emits_method_field_from_wrapper_macros".to_string())
    );
    assert_eq!(event["message"], Value::String("macro event".to_string()));
}

/// Verifies helper spans attach the optional reserved top-level fields only when present.
#[test]
fn emits_helper_api_correlation_fields_consistently() {
    let stdout = SharedBuffer::default();
    let (dispatch, _guard) = build_dispatch(
        &LoggingConfig::new(LogLevel::Info, LogOutput::Stdout, chrono_tz::UTC),
        stdout.make_writer(),
    )
    .unwrap();

    with_default(&dispatch, || {
        let span = span_with_correlation("bootstrap", Some("trace-9"), None);
        let entered = span.enter();
        tracing::info!(message = "bootstrapped");
        drop(entered);
    });

    let event = &stdout.json_lines()[0];
    assert_eq!(event["span"], Value::String("bootstrap".to_string()));
    assert_eq!(event["trace_id"], Value::String("trace-9".to_string()));
    assert_eq!(event.get("request_id"), None);
}

/// Verifies stdout-only logging writes only to the supplied stdout writer.
#[test]
fn selects_stdout_only_sink_behavior() {
    let stdout = SharedBuffer::default();
    let (dispatch, guard) = build_dispatch(
        &LoggingConfig::new(LogLevel::Info, LogOutput::Stdout, chrono_tz::UTC),
        stdout.make_writer(),
    )
    .unwrap();

    with_default(&dispatch, || {
        tracing::info!(message = "stdout only");
    });

    assert_eq!(guard.has_file_writer(), false);
    assert_eq!(stdout.json_lines().len(), 1);
}

/// Verifies file-only logging writes to the rotated file sink and keeps a worker guard alive.
#[test]
fn selects_file_only_sink_behavior() {
    let temp_dir = TempDir::new().unwrap();
    let file_config = FileLoggingConfig::new(
        temp_dir.path().join("ora.log"),
        RotationPolicy::Daily,
        NonZeroUsize::new(3).unwrap(),
    );
    let stdout = SharedBuffer::default();
    let (dispatch, guard) = build_dispatch(
        &LoggingConfig::new(LogLevel::Info, LogOutput::File(file_config), chrono_tz::UTC),
        stdout.make_writer(),
    )
    .unwrap();

    with_default(&dispatch, || {
        tracing::info!(message = "file only");
    });
    drop(dispatch);
    drop(guard);

    assert_eq!(stdout.json_lines(), Vec::<Value>::new());
    assert_eq!(read_rotated_log_lines(temp_dir.path(), "ora.log").len(), 1);
}

/// Verifies combined logging duplicates each event across stdout and the rotating file sink.
#[test]
fn selects_stdout_and_file_sink_behavior() {
    let temp_dir = TempDir::new().unwrap();
    let file_config = FileLoggingConfig::new(
        temp_dir.path().join("ora.log"),
        RotationPolicy::Daily,
        NonZeroUsize::new(3).unwrap(),
    );
    let stdout = SharedBuffer::default();
    let (dispatch, guard) = build_dispatch(
        &LoggingConfig::new(
            LogLevel::Info,
            LogOutput::StdoutAndFile(file_config),
            chrono_tz::UTC,
        ),
        stdout.make_writer(),
    )
    .unwrap();

    with_default(&dispatch, || {
        tracing::info!(message = "both sinks");
    });
    drop(dispatch);
    drop(guard);

    assert_eq!(stdout.json_lines().len(), 1);
    assert_eq!(read_rotated_log_lines(temp_dir.path(), "ora.log").len(), 1);
}

/// Verifies invalid file output configuration fails with a typed initialization error.
#[test]
fn reports_typed_initialization_failures() {
    let stdout = SharedBuffer::default();
    let error = build_dispatch(
        &LoggingConfig::new(
            LogLevel::Info,
            LogOutput::File(FileLoggingConfig::new(
                "/",
                RotationPolicy::Daily,
                NonZeroUsize::new(3).unwrap(),
            )),
            chrono_tz::UTC,
        ),
        stdout.make_writer(),
    )
    .unwrap_err();

    assert_eq!(
        match error {
            LoggingInitError::InvalidFilePath { path } => Some(path),
            _ => None,
        },
        Some("/".into())
    );
}

/// Verifies retention cleanup counts the current day's file toward the configured window.
#[test]
fn cleans_up_rotated_files_by_retention_window() {
    let temp_dir = TempDir::new().unwrap();
    create_log_file(temp_dir.path(), "ora.log.2026-05-01");
    create_log_file(temp_dir.path(), "ora.log.2026-05-02");
    create_log_file(temp_dir.path(), "ora.log.2026-05-03");
    create_log_file(temp_dir.path(), "ora.log.2026-05-04");
    create_log_file(temp_dir.path(), "unrelated.log.2026-05-01");

    cleanup_old_logs(
        &ActiveLogPath::from_path(&temp_dir.path().join("ora.log")).unwrap(),
        3,
    )
    .unwrap();

    assert_eq!(
        read_file_names(temp_dir.path()),
        vec![
            "ora.log.2026-05-02".to_string(),
            "ora.log.2026-05-03".to_string(),
            "ora.log.2026-05-04".to_string(),
            "unrelated.log.2026-05-01".to_string(),
        ]
    );
}

/// Verifies the public initializer rejects attempts to replace the configured process clock.
#[test]
fn rejects_a_second_global_initialization_attempt() {
    let lock = global_logging_test_lock();
    let _guard = lock.lock().unwrap();

    let temp_dir = TempDir::new().unwrap();
    let config = LoggingConfig::new(
        LogLevel::Info,
        LogOutput::File(FileLoggingConfig::new(
            temp_dir.path().join("ora.log"),
            RotationPolicy::Daily,
            NonZeroUsize::new(3).unwrap(),
        )),
        chrono_tz::UTC,
    );
    let _first_guard = init_logging(config.clone()).unwrap();
    assert_eq!(crate::clock::local_offset(), time::UtcOffset::UTC);
    let error = init_logging(config).unwrap_err();

    assert_eq!(
        matches!(error, LoggingInitError::ClockAlreadyInitialized),
        true
    );
}

/// Returns the shared mutex that prevents concurrent global-subscriber mutation during the one-time init test.
fn global_logging_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    LOCK.get_or_init(Mutex::default)
}

/// Creates one synthetic dated log file for retention-cleanup tests.
fn create_log_file(directory: &Path, file_name: &str) {
    fs::write(directory.join(file_name), "test\n").unwrap();
}

/// Reads the sorted file names in one directory so retention assertions can compare full outcomes.
fn read_file_names(directory: &Path) -> Vec<String> {
    let mut file_names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    file_names.sort();

    file_names
}

/// Reads every JSON line produced by one rotated log stream.
fn read_rotated_log_lines(directory: &Path, file_name_prefix: &str) -> Vec<Value> {
    let matching_file = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with(&format!("{file_name_prefix}.")))
                .unwrap_or(false)
        })
        .unwrap();
    let contents = fs::read_to_string(matching_file).unwrap();

    contents
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Captures writes in memory so sink-selection and JSON-envelope tests can inspect full emitted lines.
#[derive(Clone, Debug, Default)]
struct SharedBuffer {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl SharedBuffer {
    /// Builds the writer factory consumed by tracing's formatting layer.
    fn make_writer(&self) -> SharedBufferWriter {
        SharedBufferWriter {
            bytes: self.bytes.clone(),
        }
    }

    /// Parses every captured line as JSON so tests assert the public envelope instead of raw strings.
    fn json_lines(&self) -> Vec<Value> {
        let contents = String::from_utf8(self.bytes.lock().unwrap().clone()).unwrap();

        contents
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default()
    }
}

/// Produces append-only writers backed by the shared in-memory buffer.
#[derive(Clone, Debug)]
struct SharedBufferWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedBufferWriter {
    type Writer = SharedBufferHandle;

    /// Returns a writer handle that appends bytes into the shared buffer.
    fn make_writer(&'writer self) -> Self::Writer {
        SharedBufferHandle {
            bytes: self.bytes.clone(),
        }
    }
}

/// Appends formatted log bytes into the shared test buffer.
#[derive(Debug)]
struct SharedBufferHandle {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl std::io::Write for SharedBufferHandle {
    /// Appends every written chunk into the shared in-memory capture buffer.
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    /// Satisfies the writer contract without extra work because the buffer is purely in memory.
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
