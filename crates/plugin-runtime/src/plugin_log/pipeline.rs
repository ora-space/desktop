//! Runs the stderr → decode → filter → bounded queue → file pipeline of one process generation.
//!
//! Two tasks cooperate: the async reader owns the pipe and must never wait on anything but the
//! pipe itself, so it hands records over with `try_send` and counts what does not fit; the
//! blocking writer owns the file and drains the queue in order. Nothing here ever blocks the
//! plugin's Plugin Protocol, and nothing here ever terminates the plugin: every failure is
//! counted, reported once, and otherwise absorbed.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use ora_logging::{LogLevel, ora_info, ora_warn};
use ora_utils::text::BoundedLineFramer;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::plugin_log::record::{PluginLogRecord, RecordOrigin, decode_frame};
use crate::plugin_log::sink::PluginLogSink;

/// Longest logical record kept in memory before it is flushed as fragments.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;

/// Records the writer may lag behind the reader before new records are dropped.
pub const QUEUE_CAPACITY: usize = 1024;

/// Where one generation writes and which threshold currently applies to it.
///
/// `directory` must lie below `root`, the host-managed plugin logs root; the sink verifies
/// every level in between before it opens the active file. `level` is a live subscription
/// rather than a value: the lifecycle publishes changes to the plugin's persisted setting and
/// the reader applies them to the next record it filters, so a user never has to restart a
/// plugin to see more (or less) of its diagnostics.
#[derive(Debug)]
pub struct PluginLogSetup {
    pub root: PathBuf,
    pub directory: PathBuf,
    pub generation: u64,
    pub level: watch::Receiver<LogLevel>,
}

/// Point-in-time loss accounting of one process generation, queryable while the host runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PluginLogStats {
    pub generation: u64,
    /// Records that passed the filter but found the queue full; dropped before persistence.
    pub queue_rejected: u64,
    /// Records that reached the writer after the sink had failed; never handed to the file.
    pub sink_failed: u64,
    /// Records handed to the file whose persistence an I/O error left undecidable.
    pub indeterminate: u64,
    /// Envelopes that carried the prefix but did not parse or validate; persisted as raw.
    pub format_failures: u64,
}

/// Lock-free counters shared by the reader, the writer, and whoever queries the stats.
#[derive(Debug, Default)]
pub(crate) struct PluginLogCounters {
    generation: AtomicU64,
    queue_rejected: AtomicU64,
    sink_failed: AtomicU64,
    indeterminate: AtomicU64,
    format_failures: AtomicU64,
    queue_full_reported: AtomicBool,
    sink_failure_reported: AtomicBool,
}

impl PluginLogCounters {
    /// Snapshots the counters; the fields are independent, so the snapshot is only approximately
    /// simultaneous, which is all loss reporting needs.
    pub(crate) fn snapshot(&self) -> PluginLogStats {
        PluginLogStats {
            generation: self.generation.load(Ordering::Relaxed),
            queue_rejected: self.queue_rejected.load(Ordering::Relaxed),
            sink_failed: self.sink_failed.load(Ordering::Relaxed),
            indeterminate: self.indeterminate.load(Ordering::Relaxed),
            format_failures: self.format_failures.load(Ordering::Relaxed),
        }
    }
}

/// The running pipeline of one generation, joined by [`finish`] after the process exits.
pub(crate) struct PluginLogPipeline {
    plugin_id: String,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
    counters: Arc<PluginLogCounters>,
}

impl PluginLogPipeline {
    /// Returns the counters the runtime exposes for live queries.
    pub(crate) fn counters(&self) -> Arc<PluginLogCounters> {
        Arc::clone(&self.counters)
    }
}

/// Starts the reader and writer for one generation's stderr.
pub(crate) fn start<R>(stderr: R, plugin_id: String, setup: PluginLogSetup) -> PluginLogPipeline
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let counters = Arc::new(PluginLogCounters::default());
    counters
        .generation
        .store(setup.generation, Ordering::Relaxed);
    let (queue_tx, queue_rx) = mpsc::channel(QUEUE_CAPACITY);
    let writer = tokio::task::spawn_blocking({
        let plugin_id = plugin_id.clone();
        let counters = Arc::clone(&counters);
        let root = setup.root;
        let directory = setup.directory;
        move || run_writer(queue_rx, &root, &directory, &plugin_id, &counters)
    });
    let reader = tokio::spawn(run_reader(
        stderr,
        RecordOrigin {
            plugin_id: plugin_id.clone(),
            generation: setup.generation,
        },
        setup.level,
        queue_tx,
        Arc::clone(&counters),
    ));
    PluginLogPipeline {
        plugin_id,
        reader,
        writer,
        counters,
    }
}

/// Waits for the generation's log work to end within `deadline`, then reports its losses.
///
/// The process has already exited when this is called, but that proves nothing about the log:
/// the pipe may still hold bytes, and a grandchild that inherited the write end may keep it
/// open forever. The reader therefore gets the deadline to reach EOF and is cut off after it;
/// closing the queue then lets the writer drain and flush, again within the deadline.
pub(crate) async fn finish(pipeline: PluginLogPipeline, deadline: Duration) -> PluginLogStats {
    let PluginLogPipeline {
        plugin_id,
        mut reader,
        mut writer,
        counters,
    } = pipeline;
    let reader_completed = timeout(deadline, &mut reader).await.is_ok();
    if !reader_completed {
        // Aborting drops the queue sender, which is what lets the writer observe end of input.
        reader.abort();
        ora_warn!(
            message = "plugin stderr did not reach EOF before the log deadline; remaining output is unknown",
            plugin_id = %plugin_id,
            generation = counters.generation.load(Ordering::Relaxed),
        );
    }
    // A blocking writer cannot be aborted; past the deadline it is simply left to finish on its
    // own thread while the generation is reported without its final flush.
    let writer_completed = timeout(deadline, &mut writer).await.is_ok();
    if !writer_completed {
        ora_warn!(
            message = "plugin log writer did not finish before the deadline; queued records are unknown",
            plugin_id = %plugin_id,
            generation = counters.generation.load(Ordering::Relaxed),
        );
    }
    let stats = counters.snapshot();
    if stats.queue_rejected > 0 || stats.sink_failed > 0 || stats.indeterminate > 0 {
        ora_warn!(
            message = "plugin log generation ended with losses",
            plugin_id = %plugin_id,
            generation = stats.generation,
            queue_rejected = stats.queue_rejected,
            sink_failed = stats.sink_failed,
            indeterminate = stats.indeterminate,
            format_failures = stats.format_failures,
            drained_to_eof = reader_completed,
            flushed = writer_completed,
        );
    } else {
        ora_info!(
            message = "plugin log generation ended",
            plugin_id = %plugin_id,
            generation = stats.generation,
            format_failures = stats.format_failures,
            drained_to_eof = reader_completed,
            flushed = writer_completed,
        );
    }
    stats
}

/// Reads stderr to EOF, decoding, filtering, and enqueueing without ever awaiting the queue.
async fn run_reader<R>(
    mut stderr: R,
    origin: RecordOrigin,
    level: watch::Receiver<LogLevel>,
    queue: mpsc::Sender<PluginLogRecord>,
    counters: Arc<PluginLogCounters>,
) where
    R: AsyncRead + Unpin,
{
    let mut framer = BoundedLineFramer::new(MAX_RECORD_BYTES);
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) => break,
            Ok(length) => {
                for frame in framer.push(&buffer[..length]) {
                    submit(frame, &origin, &level, &queue, &counters);
                }
            }
            Err(error) => {
                ora_warn!(
                    message = "failed to read plugin stderr",
                    plugin_id = %origin.plugin_id,
                    generation = origin.generation,
                    error = %error,
                );
                break;
            }
        }
    }
    if let Some(frame) = framer.finish() {
        submit(frame, &origin, &level, &queue, &counters);
    }
}

/// Decodes one frame, applies the threshold in force right now, and offers it to the queue.
pub(super) fn submit(
    frame: ora_utils::text::LineFrame,
    origin: &RecordOrigin,
    level: &watch::Receiver<LogLevel>,
    queue: &mpsc::Sender<PluginLogRecord>,
    counters: &PluginLogCounters,
) {
    let decoded = decode_frame(frame, origin, ora_logging::clock::now_local());
    if decoded.format_failure.is_some() {
        counters.format_failures.fetch_add(1, Ordering::Relaxed);
    }
    // The threshold is a floor: records at or above it keep their own level. Filtering happens
    // after decoding so a structured ERROR is never squashed into a raw INFO first.
    if decoded.record.level < *level.borrow() {
        return;
    }
    match queue.try_send(decoded.record) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            counters.queue_rejected.fetch_add(1, Ordering::Relaxed);
            if !counters.queue_full_reported.swap(true, Ordering::Relaxed) {
                ora_warn!(
                    message = "plugin log queue is full; newest records are being dropped",
                    plugin_id = %origin.plugin_id,
                    generation = origin.generation,
                );
            }
        }
        // The writer only disappears when the generation is being torn down, at which point
        // the record can no longer be persisted by anyone.
        Err(mpsc::error::TrySendError::Closed(_)) => {
            counters.sink_failed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Drains the queue into the sink on a blocking thread until the reader closes it.
pub(super) fn run_writer(
    mut queue: mpsc::Receiver<PluginLogRecord>,
    root: &std::path::Path,
    directory: &std::path::Path,
    plugin_id: &str,
    counters: &PluginLogCounters,
) {
    let mut sink = match PluginLogSink::open(root, directory) {
        Ok(sink) => Some(sink),
        Err(error) => {
            report_sink_failure(plugin_id, counters, &error.to_string());
            None
        }
    };
    let mut unflushed: u64 = 0;
    while let Some(record) = queue.blocking_recv() {
        let Some(open) = sink.as_mut() else {
            counters.sink_failed.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        match open.write_line(&record.to_json_line()) {
            Ok(()) => unflushed += 1,
            Err(error) => {
                // This record and everything buffered before it may or may not have reached the
                // file; that is exactly the class the stats keep separate from "never written".
                counters
                    .indeterminate
                    .fetch_add(unflushed + 1, Ordering::Relaxed);
                unflushed = 0;
                report_sink_failure(plugin_id, counters, &error.to_string());
                sink = None;
            }
        }
        // Flush whenever the reader has nothing queued so a crash loses at most one burst.
        if queue.is_empty()
            && let Some(open) = sink.as_mut()
            && let Err(error) = open.flush()
        {
            counters
                .indeterminate
                .fetch_add(unflushed, Ordering::Relaxed);
            unflushed = 0;
            report_sink_failure(plugin_id, counters, &error.to_string());
            sink = None;
        }
    }
    if let Some(mut open) = sink
        && let Err(error) = open.flush()
    {
        counters
            .indeterminate
            .fetch_add(unflushed, Ordering::Relaxed);
        report_sink_failure(plugin_id, counters, &error.to_string());
    }
}

/// Records the sink as unavailable and tells the host log once per generation.
fn report_sink_failure(plugin_id: &str, counters: &PluginLogCounters, error: &str) {
    if !counters.sink_failure_reported.swap(true, Ordering::Relaxed) {
        ora_warn!(
            message = "plugin log sink is unavailable for this generation",
            plugin_id = %plugin_id,
            generation = counters.generation.load(Ordering::Relaxed),
            error = %error,
        );
    }
}
