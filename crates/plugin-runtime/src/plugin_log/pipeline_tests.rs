//! Behavioral tests of the plugin-log pipeline: chunking, filtering, backpressure, sink
//! failure, and bounded teardown.

use std::sync::Arc;
use std::time::Duration;

use ora_logging::LogLevel;
use ora_utils::text::LineFrame;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncWriteExt, duplex};
use tokio::sync::{mpsc, watch};

use super::pipeline::{PluginLogCounters, submit};
use super::record::RecordOrigin;
use super::{PluginLogSetup, PluginLogStats, finish, start};

/// The plugin's log directory below the test's logs root.
fn log_directory(temp: &std::path::Path) -> std::path::PathBuf {
    temp.join("logs").join("official").join("example")
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

/// Starts a pipeline over a duplex pipe and returns the write half the test drives.
fn pipeline_over_pipe(
    directory: &std::path::Path,
    level: watch::Receiver<LogLevel>,
) -> (tokio::io::DuplexStream, super::PluginLogPipeline) {
    let (writer, reader) = duplex(64);
    let pipeline = start(
        reader,
        "official/example".to_string(),
        PluginLogSetup {
            root: directory.join("logs"),
            directory: log_directory(directory),
            generation: 2,
            level,
        },
    );
    (writer, pipeline)
}

/// Records split across arbitrary pipe reads, a trailing unterminated fragment, structured and
/// raw content all land in order with host identity, and the generation ends without losses.
#[tokio::test(flavor = "multi_thread")]
async fn persists_structured_and_raw_records_independently_of_read_boundaries() {
    ora_logging::initialize_test_clock();
    let temp = TempDir::new().expect("temp dir");
    let (_level_tx, level) = watch::channel(LogLevel::Info);
    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), level);
    let input = concat!(
        "@ora/plugin-log/v1 {\"level\":\"ERROR\",\"message\":\"multi\\nline\",\"context\":{\"plugin_id\":\"spoof\"}}\n",
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

    let stats = finish(pipeline, Duration::from_secs(5)).await;

    assert_eq!(
        stats,
        PluginLogStats {
            generation: 2,
            ..PluginLogStats::default()
        }
    );
    assert_eq!(
        persisted_records(&log_directory(temp.path())),
        vec![
            json!({
                "level": "ERROR",
                "target": "plugin",
                "message": "multi\nline",
                "context": { "plugin_id": "official/example", "generation": 2 },
            }),
            json!({
                "level": "INFO",
                "target": "plugin.stderr",
                "message": "[plugin:error] legacy",
                "context": { "plugin_id": "official/example", "generation": 2 },
            }),
            json!({
                "level": "INFO",
                "target": "plugin.stderr",
                "message": "trailing fragment",
                "context": { "plugin_id": "official/example", "generation": 2 },
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
    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), level);
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
    let stats = finish(pipeline, Duration::from_secs(5)).await;

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
        (levels_and_messages, stats.queue_rejected, stats.sink_failed),
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

/// A full queue rejects the newest records, keeps the earliest in order, counts each rejection
/// once, and reports the condition to the host log only on its first occurrence.
#[tokio::test]
async fn a_full_queue_drops_the_newest_records_and_counts_them() {
    ora_logging::initialize_test_clock();
    let (_level_tx, level) = watch::channel(LogLevel::Info);
    let (queue_tx, mut queue_rx) = mpsc::channel(2);
    let counters = Arc::new(PluginLogCounters::default());
    let origin = RecordOrigin {
        plugin_id: "official/example".to_string(),
        generation: 1,
    };
    for index in 0..5 {
        submit(
            LineFrame::Line(format!("record {index}").into_bytes()),
            &origin,
            &level,
            &queue_tx,
            &counters,
        );
    }
    // Policy-filtered records never reach the queue and must not count as rejected.
    submit(
        LineFrame::Line(b"@ora/plugin-log/v1 {\"level\":\"DEBUG\",\"message\":\"x\"}".to_vec()),
        &origin,
        &level,
        &queue_tx,
        &counters,
    );

    let queued = [
        queue_rx.recv().await.expect("first").message,
        queue_rx.recv().await.expect("second").message,
    ];
    assert_eq!(
        (queued, counters.snapshot()),
        (
            ["record 0".to_string(), "record 1".to_string()],
            PluginLogStats {
                generation: 0,
                queue_rejected: 3,
                ..PluginLogStats::default()
            }
        )
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
    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), level);
    // Far more than the pipe buffer: the test only completes if the reader keeps draining.
    for index in 0..500 {
        stderr
            .write_all(format!("line {index}\n").as_bytes())
            .await
            .expect("write");
    }
    drop(stderr);

    let stats = finish(pipeline, Duration::from_secs(5)).await;

    assert_eq!(
        (
            stats,
            std::fs::read_to_string(temp.path().join("logs").join("official"))
                .expect("foreign file intact")
        ),
        (
            PluginLogStats {
                generation: 2,
                sink_failed: 500,
                ..PluginLogStats::default()
            },
            "foreign file".to_string()
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
    let (mut stderr, pipeline) = pipeline_over_pipe(temp.path(), level);
    stderr.write_all(b"before exit\n").await.expect("write");
    wait_for_records(&log_directory(temp.path()), 1).await;

    let started = std::time::Instant::now();
    let stats = tokio::time::timeout(
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
            stats.queue_rejected,
            persisted_records(&log_directory(temp.path()))
                .into_iter()
                .map(|record| record["message"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        ),
        (0, vec!["before exit".to_string()])
    );
}
