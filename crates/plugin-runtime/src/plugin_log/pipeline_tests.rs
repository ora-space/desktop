//! Behavioral tests of the plugin-log pipeline: chunking, filtering, backpressure by count and
//! by bytes, sink failure classes, writer exclusivity, and bounded single-deadline teardown.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ora_logging::LogLevel;
use ora_utils::text::LineFrame;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncWriteExt, duplex};
use tokio::sync::{mpsc, watch};

use super::pipeline::{PluginLogCounters, run_writer, start_with_sink, submit};
use super::record::RecordOrigin;
use super::sink::{LineSink, PluginLogSink, SinkOpenError};
use super::{
    MAX_QUEUE_BYTES, MAX_RECORD_BYTES, PluginLogSetup, PluginLogStats, PluginLogTeardown, finish,
    start,
};

const PLUGIN_ID: &str = "official/example";
const SESSION: &str = "session-1";

/// The plugin's log directory below the test's logs root.
fn log_directory(temp: &std::path::Path) -> std::path::PathBuf {
    temp.join("logs").join("official").join("example")
}

/// The identity every record of generation `generation` carries.
fn host_context(generation: u64) -> Value {
    json!({ "plugin_id": PLUGIN_ID, "host_session_id": SESSION, "generation": generation })
}

fn origin(generation: u64) -> RecordOrigin {
    RecordOrigin {
        plugin_id: PLUGIN_ID.to_string(),
        host_session_id: SESSION.to_string(),
        generation,
    }
}

/// Reads back every persisted line as JSON with the host timestamp removed, since only its
/// presence is stable across runs.
fn persisted_records(directory: &std::path::Path) -> Vec<Value> {
    let content = std::fs::read_to_string(directory.join("plugin.log")).unwrap_or_default();
    content
        .lines()
        .map(|line| {
            let mut value: Value = serde_json::from_str(line).expect("json line");
            assert!(value["timestamp"].is_string(), "timestamp stamped by host");
            value.as_object_mut().expect("object").remove("timestamp");
            value
        })
        .collect()
}

/// Messages of every persisted record, in order.
fn persisted_messages(directory: &std::path::Path) -> Vec<String> {
    persisted_records(directory)
        .into_iter()
        .map(|record| record["message"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Starts a pipeline over a duplex pipe and returns the write half the test drives.
fn pipeline_over_pipe(
    directory: &std::path::Path,
    generation: u64,
    level: watch::Receiver<LogLevel>,
) -> (tokio::io::DuplexStream, super::PluginLogPipeline) {
    let (writer, reader) = duplex(64);
    let pipeline = start(
        reader,
        PLUGIN_ID.to_string(),
        PluginLogSetup {
            root: directory.join("logs"),
            directory: log_directory(directory),
            host_session_id: SESSION.to_string(),
            generation,
            level,
        },
    );
    (writer, pipeline)
}

/// Polls the active file until it holds `count` lines; the writer flushes whenever idle.
async fn wait_for_records(directory: &std::path::Path, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let lines = std::fs::read_to_string(directory.join("plugin.log"))
                .map(|content| content.lines().count())
                .unwrap_or(0);
            if lines >= count {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("records were persisted in time");
}

/// Records split across arbitrary pipe reads, a trailing unterminated fragment, structured and
/// raw content all land in order with host identity, and the generation ends clean.
#[tokio::test(flavor = "multi_thread")]
async fn persists_structured_and_raw_records_independently_of_read_boundaries() {
    ora_logging::initialize_test_clock();
    let temp = TempDir::new().expect("temp dir");
    let (_level_tx, level) = watch::channel(LogLevel::Info);
    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), 2, level);
    let input = concat!(
        "@ora/plugin-log/v1 {\"level\":\"ERROR\",\"message\":\"multi\\nline\",\"context\":{\"plugin_id\":\"spoof\",\"host_session_id\":\"spoof\"}}\n",
        "[plugin:error] legacy\r\n",
        "@ora/plugin-log/v1 {\"level\":\"DEBUG\",\"message\":\"filtered\"}\n",
        "trailing fragment"
    )
    .as_bytes();
    // Three-byte writes cut the envelope prefix, the JSON, and the CRLF at awkward places.
    for chunk in input.chunks(3) {
        stderr.write_all(chunk).await.expect("write chunk");
    }
    drop(stderr);

    let teardown = finish(pipeline, Duration::from_secs(5)).await;

    assert_eq!(
        teardown,
        PluginLogTeardown {
            stats: PluginLogStats {
                generation: 2,
                accepted: 3,
                ..PluginLogStats::default()
            },
            stderr_reached_eof: true,
            writer_released: true,
            queued_at_deadline: 0,
        }
    );
    assert_eq!(
        persisted_records(&log_directory(temp.path())),
        vec![
            json!({
                "level": "ERROR",
                "target": "plugin",
                "message": "multi\nline",
                "context": host_context(2),
            }),
            json!({
                "level": "INFO",
                "target": "plugin.stderr",
                "message": "[plugin:error] legacy",
                "context": host_context(2),
            }),
            json!({
                "level": "INFO",
                "target": "plugin.stderr",
                "message": "trailing fragment",
                "context": host_context(2),
            }),
        ]
    );
}

/// The threshold is a floor applied per record at filter time: raising it mid-generation stops
/// lower levels immediately without a restart, lowering it lets them through again, records that
/// pass keep their own level, and policy-filtered raw text is not counted as loss.
#[tokio::test(flavor = "multi_thread")]
async fn applies_the_live_threshold_as_a_floor_without_restart() {
    ora_logging::initialize_test_clock();
    let temp = TempDir::new().expect("temp dir");
    let (level_tx, level) = watch::channel(LogLevel::Info);
    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), 2, level);
    let record = |level: &str, message: &str| {
        format!("@ora/plugin-log/v1 {{\"level\":\"{level}\",\"message\":\"{message}\"}}\n")
    };
    let mut phase = String::new();
    phase.push_str(&record("WARN", "warn-at-info"));
    phase.push_str("raw-at-info\n");
    stderr.write_all(phase.as_bytes()).await.expect("phase 1");
    // Wait until the reader has consumed phase 1 before changing the threshold so the change
    // is observably ordered after those records.
    wait_for_records(&log_directory(temp.path()), 2).await;

    level_tx.send_replace(LogLevel::Error);
    let mut phase = String::new();
    phase.push_str(&record("WARN", "warn-at-error"));
    phase.push_str("raw-at-error\n");
    phase.push_str(&record("ERROR", "error-at-error"));
    stderr.write_all(phase.as_bytes()).await.expect("phase 2");
    wait_for_records(&log_directory(temp.path()), 3).await;

    level_tx.send_replace(LogLevel::Debug);
    stderr
        .write_all(record("DEBUG", "debug-at-debug").as_bytes())
        .await
        .expect("phase 3");
    drop(stderr);
    let teardown = finish(pipeline, Duration::from_secs(5)).await;

    let levels_and_messages = persisted_records(&log_directory(temp.path()))
        .into_iter()
        .map(|record| {
            (
                record["level"].as_str().unwrap_or_default().to_string(),
                record["message"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        (
            levels_and_messages,
            teardown.stats.queue_rejected,
            teardown.stats.sink_failed
        ),
        (
            vec![
                ("WARN".to_string(), "warn-at-info".to_string()),
                ("INFO".to_string(), "raw-at-info".to_string()),
                ("ERROR".to_string(), "error-at-error".to_string()),
                ("DEBUG".to_string(), "debug-at-debug".to_string()),
            ],
            0,
            0
        )
    );
}

/// A full queue rejects the newest records, keeps the earliest in order, counts each rejection
/// once, and reports the condition to the host log only on its first occurrence.
#[tokio::test]
async fn a_full_queue_drops_the_newest_records_and_counts_them() {
    ora_logging::initialize_test_clock();
    let (_level_tx, level) = watch::channel(LogLevel::Info);
    let (queue_tx, mut queue_rx) = mpsc::channel(2);
    let counters = Arc::new(PluginLogCounters::default());
    for index in 0..5 {
        submit(
            LineFrame::Line(format!("record {index}").into_bytes()),
            &origin(1),
            &level,
            &queue_tx,
            &counters,
        );
    }
    // Policy-filtered records never reach the queue and must not count as rejected.
    submit(
        LineFrame::Line(b"@ora/plugin-log/v1 {\"level\":\"DEBUG\",\"message\":\"x\"}".to_vec()),
        &origin(1),
        &level,
        &queue_tx,
        &counters,
    );

    let message_of = |line: String| -> String {
        serde_json::from_str::<Value>(line.trim_end()).expect("json")["message"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    let queued = [
        message_of(queue_rx.recv().await.expect("first")),
        message_of(queue_rx.recv().await.expect("second")),
    ];
    assert_eq!(
        (queued, counters.snapshot(), counters.host_warnings()),
        (
            ["record 0".to_string(), "record 1".to_string()],
            PluginLogStats {
                generation: 0,
                accepted: 2,
                queue_rejected: 3,
                ..PluginLogStats::default()
            },
            1
        )
    );
}

/// The queue is bounded in bytes as well as in count: maximal records fill it long before
/// 1024 of them are queued, and the rejected remainder is counted as queue loss.
#[tokio::test]
async fn the_queue_is_bounded_by_bytes_as_well_as_count() {
    ora_logging::initialize_test_clock();
    let (_level_tx, level) = watch::channel(LogLevel::Info);
    let (queue_tx, queue_rx) = mpsc::channel(super::QUEUE_CAPACITY);
    let counters = Arc::new(PluginLogCounters::default());
    let attempts = 200_u64;
    for _ in 0..attempts {
        submit(
            LineFrame::Line(vec![b'a'; MAX_RECORD_BYTES]),
            &origin(1),
            &level,
            &queue_tx,
            &counters,
        );
    }

    let stats = counters.snapshot();
    let accepted_bytes = usize::try_from(stats.accepted).expect("fits") * MAX_RECORD_BYTES;
    assert_eq!(
        (
            stats.accepted + stats.queue_rejected,
            stats.queue_rejected > 0,
            accepted_bytes <= MAX_QUEUE_BYTES,
            // One more maximal record would have crossed the bound, so the queue really is full.
            accepted_bytes + MAX_RECORD_BYTES > MAX_QUEUE_BYTES - MAX_RECORD_BYTES,
        ),
        (attempts, true, true, true)
    );
    drop(queue_rx);
}

/// The worst-case rendering of one maximal raw record — every byte invalid UTF-8 — stays within
/// a fixed multiple of the record limit, which is what makes `MAX_QUEUE_BYTES` a real bound.
#[test]
fn a_maximal_raw_record_renders_within_a_fixed_multiple_of_the_limit() {
    let decoded = super::record::decode_frame(
        LineFrame::Line(vec![0xFF; MAX_RECORD_BYTES]),
        &origin(1),
        time::macros::datetime!(2026-09-14 10:00:00 +08:00),
    );
    let rendered = decoded.record.to_json_line().len();
    // `\xFF` is four characters, each backslash doubles under JSON escaping: eight bytes per
    // input byte, plus the envelope fields.
    assert!(
        rendered <= 8 * MAX_RECORD_BYTES + 1024,
        "rendered {rendered} bytes"
    );
}

/// A foreign path under the reserved name fails the sink, is left untouched, and the reader
/// keeps draining while every filtered-in record is counted against the sink.
#[tokio::test(flavor = "multi_thread")]
async fn a_conflicting_log_path_fails_the_sink_but_keeps_draining() {
    ora_logging::initialize_test_clock();
    let temp = TempDir::new().expect("temp dir");
    // A file where the namespace directory should be: the sink must not replace it.
    std::fs::create_dir_all(temp.path().join("logs")).expect("logs root");
    std::fs::write(temp.path().join("logs").join("official"), "foreign file").expect("foreign");
    let (_level_tx, level) = watch::channel(LogLevel::Info);
    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), 2, level);
    // Far more than the pipe buffer: the test only completes if the reader keeps draining.
    for index in 0..500 {
        stderr
            .write_all(format!("line {index}\n").as_bytes())
            .await
            .expect("write");
    }
    drop(stderr);

    let teardown = finish(pipeline, Duration::from_secs(5)).await;

    assert_eq!(
        (
            teardown.stats,
            std::fs::read_to_string(temp.path().join("logs").join("official"))
                .expect("foreign file intact")
        ),
        (
            PluginLogStats {
                generation: 2,
                accepted: 500,
                sink_failed: 500,
                ..PluginLogStats::default()
            },
            "foreign file".to_string()
        )
    );
}

/// A sink whose N-th write (or whose flush) fails, standing in for a disk error.
struct FaultySink {
    written: Vec<String>,
    fail_write_at: Option<usize>,
    fail_flush: bool,
}

impl LineSink for FaultySink {
    fn write_line(&mut self, line: &str) -> io::Result<()> {
        if self.fail_write_at == Some(self.written.len()) {
            return Err(io::Error::other("injected write failure"));
        }
        self.written.push(line.to_string());
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            return Err(io::Error::other("injected flush failure"));
        }
        Ok(())
    }
}

/// Feeds `lines` through `run_writer` over `sink` on a blocking thread and returns the counters.
async fn drain_through(sink: FaultySink, lines: &[&str]) -> Arc<PluginLogCounters> {
    let counters = Arc::new(PluginLogCounters::default());
    let (queue_tx, queue_rx) = mpsc::channel(64);
    for line in lines {
        queue_tx.send((*line).to_string()).await.expect("queue");
    }
    drop(queue_tx);
    let writer_counters = Arc::clone(&counters);
    tokio::task::spawn_blocking(move || {
        run_writer(queue_rx, || Ok(sink), PLUGIN_ID, &writer_counters);
    })
    .await
    .expect("writer thread");
    counters
}

/// A write failure makes the failed record and everything unflushed before it indeterminate,
/// every later record a known sink loss, and warns the host exactly once.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_failure_splits_indeterminate_from_lost_and_warns_once() {
    ora_logging::initialize_test_clock();
    let lines = ["a\n", "b\n", "c\n", "d\n", "e\n"];
    // Everything is queued before the writer starts, so no idle flush happens before the fault.
    let counters = drain_through(
        FaultySink {
            written: Vec::new(),
            fail_write_at: Some(2),
            fail_flush: false,
        },
        &lines,
    )
    .await;

    assert_eq!(
        (counters.snapshot(), counters.host_warnings()),
        (
            PluginLogStats {
                indeterminate: 3,
                sink_failed: 2,
                ..PluginLogStats::default()
            },
            1
        )
    );
}

/// A flush failure turns every record written since the last good flush into an indeterminate
/// outcome rather than a claimed success, and the sink is not retried.
#[tokio::test(flavor = "multi_thread")]
async fn a_flush_failure_makes_written_records_indeterminate() {
    ora_logging::initialize_test_clock();
    let counters = drain_through(
        FaultySink {
            written: Vec::new(),
            fail_write_at: None,
            fail_flush: true,
        },
        &["a\n", "b\n", "c\n"],
    )
    .await;

    // The idle flush after the last dequeue fails with all three records unflushed.
    assert_eq!(
        (counters.snapshot(), counters.host_warnings()),
        (
            PluginLogStats {
                indeterminate: 3,
                ..PluginLogStats::default()
            },
            1
        )
    );
}

/// While an earlier writer still holds the active file, the next generation runs with its sink
/// unavailable — draining, counting, never opening the file beside the old writer — and only a
/// generation started after the release persists again.
#[tokio::test(flavor = "multi_thread")]
async fn a_generation_never_writes_beside_an_unreleased_writer() {
    ora_logging::initialize_test_clock();
    let temp = TempDir::new().expect("temp dir");
    let directory = log_directory(temp.path());
    let held = PluginLogSink::open(&temp.path().join("logs"), &directory).expect("old writer");
    let (_level_tx, level) = watch::channel(LogLevel::Info);

    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), 2, level.clone());
    stderr.write_all(b"while held\n").await.expect("write");
    drop(stderr);
    let blocked = finish(pipeline, Duration::from_secs(5)).await;
    drop(held);

    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), 3, level);
    stderr.write_all(b"after release\n").await.expect("write");
    drop(stderr);
    let released = finish(pipeline, Duration::from_secs(5)).await;

    assert_eq!(
        (
            blocked.stats,
            released.stats,
            persisted_messages(&directory),
        ),
        (
            PluginLogStats {
                generation: 2,
                accepted: 1,
                sink_failed: 1,
                ..PluginLogStats::default()
            },
            PluginLogStats {
                generation: 3,
                accepted: 1,
                ..PluginLogStats::default()
            },
            vec!["after release".to_string()],
        )
    );
}

/// A sink whose first write blocks until the test releases it, standing in for stuck file I/O.
struct BlockingSink {
    gate: std::sync::mpsc::Receiver<()>,
    written: Arc<AtomicUsize>,
}

impl LineSink for BlockingSink {
    fn write_line(&mut self, _line: &str) -> io::Result<()> {
        if self.written.fetch_add(1, Ordering::SeqCst) == 0 {
            let _ = self.gate.recv();
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Reader and writer share one deadline: with stderr never closing and the writer stuck,
/// teardown returns after roughly one deadline (not one per stage), reports the unknown remainder
/// and the known queued-but-uncommitted count separately, and the writer keeps running until it
/// is really released.
#[tokio::test(flavor = "multi_thread")]
async fn teardown_shares_one_deadline_and_reports_the_unreleased_writer() {
    ora_logging::initialize_test_clock();
    let (gate_tx, gate_rx) = std::sync::mpsc::channel();
    let written = Arc::new(AtomicUsize::new(0));
    let (_level_tx, level) = watch::channel(LogLevel::Info);
    let (mut stderr, reader) = duplex(64);
    let pipeline = start_with_sink(reader, PLUGIN_ID.to_string(), origin(4), level, {
        let written = Arc::clone(&written);
        move || {
            Ok(BlockingSink {
                gate: gate_rx,
                written,
            })
        }
    });
    stderr.write_all(b"one\ntwo\nthree\n").await.expect("write");
    // Let the writer take the first line and block on it before teardown starts.
    tokio::time::timeout(Duration::from_secs(5), async {
        while written.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("writer took the first record");

    let deadline = Duration::from_secs(1);
    let started = std::time::Instant::now();
    let teardown = finish(pipeline, deadline).await;
    let elapsed = started.elapsed();
    // `stderr` is still open here: the write end was never dropped.
    gate_tx.send(()).expect("release the writer");
    drop(stderr);

    assert_eq!(
        (
            teardown,
            // The reader is cut at three quarters of the deadline; the writer gets the rest.
            elapsed >= deadline.mul_f64(0.75),
            // Two full deadlines would be ~2 s; one shared deadline lands well under that.
            elapsed < deadline + deadline / 2,
        ),
        (
            PluginLogTeardown {
                stats: PluginLogStats {
                    generation: 4,
                    accepted: 3,
                    ..PluginLogStats::default()
                },
                stderr_reached_eof: false,
                writer_released: false,
                queued_at_deadline: 2,
            },
            true,
            true,
        )
    );
}

/// When stderr never reaches EOF, teardown still returns within the deadline and what was read
/// before the cut-off is persisted.
#[tokio::test(flavor = "multi_thread")]
async fn teardown_is_bounded_when_stderr_never_closes() {
    ora_logging::initialize_test_clock();
    let temp = TempDir::new().expect("temp dir");
    let (_level_tx, level) = watch::channel(LogLevel::Info);
    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), 2, level);
    stderr.write_all(b"before exit\n").await.expect("write");
    wait_for_records(&log_directory(temp.path()), 1).await;

    let started = std::time::Instant::now();
    let teardown = tokio::time::timeout(
        Duration::from_secs(5),
        finish(pipeline, Duration::from_millis(200)),
    )
    .await
    .expect("finish returns within the deadline");
    // `stderr` is still open here: the write end was never dropped.
    drop(stderr);

    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(
        (
            teardown.stderr_reached_eof,
            teardown.writer_released,
            teardown.queued_at_deadline,
            persisted_messages(&log_directory(temp.path())),
        ),
        (false, true, 0, vec!["before exit".to_string()])
    );
}

/// The injected-opener seam rejects like the real one: an opener error fails the sink up front
/// and the generation drains with every record counted as lost.
#[tokio::test(flavor = "multi_thread")]
async fn an_opener_error_fails_the_generation_up_front() {
    ora_logging::initialize_test_clock();
    let (_level_tx, level) = watch::channel(LogLevel::Info);
    let (mut stderr, reader) = duplex(64);
    let pipeline = start_with_sink(reader, PLUGIN_ID.to_string(), origin(5), level, || {
        Err::<FaultySink, _>(SinkOpenError::Busy {
            path: std::path::PathBuf::from("plugin.log.lock"),
        })
    });
    stderr.write_all(b"a\nb\n").await.expect("write");
    drop(stderr);

    let teardown = finish(pipeline, Duration::from_secs(5)).await;

    assert_eq!(
        teardown.stats,
        PluginLogStats {
            generation: 5,
            accepted: 2,
            sink_failed: 2,
            ..PluginLogStats::default()
        }
    );
}
