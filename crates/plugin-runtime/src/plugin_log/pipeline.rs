//! Runs the stderr → decode → filter → bounded queue → file pipeline of one process generation.
//!
//! Two tasks cooperate: the async reader owns the pipe and must never wait on anything but the
//! pipe itself, so it renders each record, hands the line over with `try_send`, and counts what
//! does not fit; the blocking writer owns the file and drains the queue in order. Nothing here
//! ever blocks the plugin's Plugin Protocol, and nothing here ever terminates the plugin: every
//! failure is counted, reported once, and otherwise absorbed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use ora_logging::{LogLevel, ora_info, ora_warn};
use ora_utils::text::BoundedLineFramer;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout_at};

use crate::plugin_log::record::{RecordOrigin, decode_frame};
use crate::plugin_log::sink::{LineSink, PluginLogSink, SinkOpenError};

/// Longest logical record kept in memory before it is flushed as fragments.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;

/// Records the writer may lag behind the reader before new records are dropped.
pub const QUEUE_CAPACITY: usize = 1024;

/// Fraction of the teardown deadline the stderr reader may consume before it is cut off, the
/// rest being left for the queue drain and flush.
const READER_SHARE: f64 = 0.75;

/// Rendered bytes the queue may hold at once, whatever the record count.
///
/// A raw record of [`MAX_RECORD_BYTES`] invalid bytes renders to roughly eight times its size
/// once escaped and JSON-encoded, so a count limit alone would not bound memory; this does.
pub const MAX_QUEUE_BYTES: usize = 8 * 1024 * 1024;

/// Where one generation writes and which threshold currently applies to it.
///
/// `directory` must lie below `root`, the host-managed plugin logs root; the sink verifies
/// every level in between before it opens the active file. `host_session_id` is the identity of
/// this host run and `generation` the launch count within it; both are stamped on every record.
/// `level` is a live subscription rather than a value: the lifecycle publishes changes to the
/// plugin's persisted setting and the reader applies them to the next record it filters, so a
/// user never has to restart a plugin to see more (or less) of its diagnostics.
#[derive(Debug)]
pub struct PluginLogSetup {
    pub root: PathBuf,
    pub directory: PathBuf,
    pub host_session_id: String,
    pub generation: u64,
    pub level: watch::Receiver<LogLevel>,
}

/// Point-in-time loss accounting of one process generation, queryable while the host runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PluginLogStats {
    pub generation: u64,
    /// Records that passed the filter and entered the queue.
    pub accepted: u64,
    /// Records that passed the filter but found the queue full; dropped before persistence.
    pub queue_rejected: u64,
    /// Records that reached the writer after the sink had failed; never handed to the file.
    pub sink_failed: u64,
    /// Records handed to the file whose persistence an I/O error left undecidable.
    pub indeterminate: u64,
    /// Envelopes that carried the prefix but did not parse or validate; persisted as raw.
    pub format_failures: u64,
}

/// What the teardown of one generation established before its single deadline ran out.
///
/// The three loss classes are deliberately separate: `stats` counts what the pipeline knows it
/// lost, `queued_at_deadline` counts records that were accepted but never reached the sink, and
/// a `false` in either flag stands for a quantity nobody can know — bytes still in the pipe, or
/// lines a writer may or may not have landed after the host stopped waiting for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLogTeardown {
    pub stats: PluginLogStats,
    /// stderr reached EOF within the deadline; otherwise unread output of unknown size was left.
    pub stderr_reached_eof: bool,
    /// The writer flushed and released the active file within the deadline.
    pub writer_released: bool,
    /// Records accepted into the queue that the writer had not dequeued when the deadline passed.
    pub queued_at_deadline: u64,
}

/// Lock-free counters shared by the reader, the writer, and whoever queries the stats.
#[derive(Debug, Default)]
pub(crate) struct PluginLogCounters {
    generation: AtomicU64,
    accepted: AtomicU64,
    dequeued: AtomicU64,
    queue_rejected: AtomicU64,
    sink_failed: AtomicU64,
    indeterminate: AtomicU64,
    format_failures: AtomicU64,
    queued_bytes: AtomicUsize,
    queue_full_reported: AtomicBool,
    sink_failure_reported: AtomicBool,
    host_warnings: AtomicU64,
}

impl PluginLogCounters {
    /// Snapshots the counters; the fields are independent, so the snapshot is only approximately
    /// simultaneous, which is all loss reporting needs.
    pub(crate) fn snapshot(&self) -> PluginLogStats {
        PluginLogStats {
            generation: self.generation.load(Ordering::Relaxed),
            accepted: self.accepted.load(Ordering::Relaxed),
            queue_rejected: self.queue_rejected.load(Ordering::Relaxed),
            sink_failed: self.sink_failed.load(Ordering::Relaxed),
            indeterminate: self.indeterminate.load(Ordering::Relaxed),
            format_failures: self.format_failures.load(Ordering::Relaxed),
        }
    }

    /// Number of times this generation wrote a first-occurrence warning to the Ora runtime log;
    /// the reporting contract caps it at one per failure class, never one per lost record.
    #[cfg(test)]
    pub(super) fn host_warnings(&self) -> u64 {
        self.host_warnings.load(Ordering::Relaxed)
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

/// Starts the reader and writer for one generation's stderr over the production file sink.
pub(crate) fn start<R>(stderr: R, plugin_id: String, setup: PluginLogSetup) -> PluginLogPipeline
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let origin = RecordOrigin {
        plugin_id: plugin_id.clone(),
        host_session_id: setup.host_session_id,
        generation: setup.generation,
    };
    let root = setup.root;
    let directory = setup.directory;
    let opener_plugin_id = plugin_id.clone();
    start_with_sink(stderr, plugin_id, origin, setup.level, move || {
        open_file_sink(&root, &directory, &opener_plugin_id)
    })
}

/// Starts the reader and writer with an injected sink opener, which is how tests substitute a
/// failing or blocking sink for the file.
pub(super) fn start_with_sink<R, S>(
    stderr: R,
    plugin_id: String,
    origin: RecordOrigin,
    level: watch::Receiver<LogLevel>,
    open: impl FnOnce() -> Result<S, SinkOpenError> + Send + 'static,
) -> PluginLogPipeline
where
    R: AsyncRead + Unpin + Send + 'static,
    S: LineSink,
{
    let counters = Arc::new(PluginLogCounters::default());
    counters
        .generation
        .store(origin.generation, Ordering::Relaxed);
    let (queue_tx, queue_rx) = mpsc::channel(QUEUE_CAPACITY);
    let writer = tokio::task::spawn_blocking({
        let plugin_id = plugin_id.clone();
        let counters = Arc::clone(&counters);
        move || run_writer(queue_rx, open, &plugin_id, &counters)
    });
    let reader = tokio::spawn(run_reader(
        stderr,
        origin,
        level,
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

/// Waits for the generation's log work to end within one shared `deadline`, then reports.
///
/// The process has already exited when this is called, but that proves nothing about the log:
/// the pipe may still hold bytes, and a grandchild that inherited the write end may keep it
/// open forever. Reader, queue drain, and flush share a single point in time: the reader is cut
/// off at [`READER_SHARE`] of it, which closes the queue and lets the writer drain and flush
/// within the remainder of the same deadline — never a fresh one. The reader does not get the
/// whole deadline because a pipe that never closes is the common failure, and it must not
/// leave the writer no time at all to land what was already queued.
///
/// A writer that misses the deadline cannot be aborted (it is a blocking thread) and is not
/// forgotten either: it keeps the writer lock until it really finishes, so the next generation
/// finds the sink busy instead of writing beside it, and a delete-data uninstall is refused.
pub(crate) async fn finish(pipeline: PluginLogPipeline, deadline: Duration) -> PluginLogTeardown {
    let PluginLogPipeline {
        plugin_id,
        mut reader,
        mut writer,
        counters,
    } = pipeline;
    let deadline_at = Instant::now() + deadline;
    let reader_deadline_at = deadline_at - deadline.mul_f64(1.0 - READER_SHARE);
    let stderr_reached_eof = timeout_at(reader_deadline_at, &mut reader).await.is_ok();
    if !stderr_reached_eof {
        // Aborting drops the queue sender, which is what lets the writer observe end of input.
        reader.abort();
        ora_warn!(
            message = "plugin stderr did not reach EOF before the log deadline; remaining output is unknown",
            plugin_id = %plugin_id,
            generation = counters.generation.load(Ordering::Relaxed),
        );
    }
    let writer_released = timeout_at(deadline_at, &mut writer).await.is_ok();
    let stats = counters.snapshot();
    let queued_at_deadline = stats
        .accepted
        .saturating_sub(counters.dequeued.load(Ordering::Relaxed));
    if !writer_released {
        ora_warn!(
            message = "plugin log writer did not release the file before the deadline; it keeps the writer lock until it finishes",
            plugin_id = %plugin_id,
            generation = stats.generation,
            queued_at_deadline,
        );
    }
    if stats.queue_rejected > 0
        || stats.sink_failed > 0
        || stats.indeterminate > 0
        || queued_at_deadline > 0
    {
        ora_warn!(
            message = "plugin log generation ended with losses",
            plugin_id = %plugin_id,
            generation = stats.generation,
            accepted = stats.accepted,
            queue_rejected = stats.queue_rejected,
            sink_failed = stats.sink_failed,
            indeterminate = stats.indeterminate,
            queued_at_deadline,
            format_failures = stats.format_failures,
            drained_to_eof = stderr_reached_eof,
            flushed = writer_released,
        );
    } else {
        ora_info!(
            message = "plugin log generation ended",
            plugin_id = %plugin_id,
            generation = stats.generation,
            accepted = stats.accepted,
            format_failures = stats.format_failures,
            drained_to_eof = stderr_reached_eof,
            flushed = writer_released,
        );
    }
    PluginLogTeardown {
        stats,
        stderr_reached_eof,
        writer_released,
        queued_at_deadline,
    }
}

/// Reads stderr to EOF, decoding, filtering, and enqueueing without ever awaiting the queue.
async fn run_reader<R>(
    mut stderr: R,
    origin: RecordOrigin,
    level: watch::Receiver<LogLevel>,
    queue: mpsc::Sender<String>,
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

/// Decodes one frame, applies the threshold in force right now, renders the line, and offers
/// it to the queue under both the count and the byte bound.
pub(super) fn submit(
    frame: ora_utils::text::LineFrame,
    origin: &RecordOrigin,
    level: &watch::Receiver<LogLevel>,
    queue: &mpsc::Sender<String>,
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
    let line = decoded.record.to_json_line();
    let bytes = line.len();
    // Reserve the bytes before offering the line so two views of "full" cannot both admit it.
    let previously_queued = counters.queued_bytes.fetch_add(bytes, Ordering::Relaxed);
    if previously_queued + bytes > MAX_QUEUE_BYTES {
        counters.queued_bytes.fetch_sub(bytes, Ordering::Relaxed);
        reject_for_full_queue(origin, counters);
        return;
    }
    match queue.try_send(line) {
        Ok(()) => {
            counters.accepted.fetch_add(1, Ordering::Relaxed);
        }
        Err(mpsc::error::TrySendError::Full(_)) => {
            counters.queued_bytes.fetch_sub(bytes, Ordering::Relaxed);
            reject_for_full_queue(origin, counters);
        }
        // The writer only disappears when the generation is being torn down, at which point
        // the record can no longer be persisted by anyone.
        Err(mpsc::error::TrySendError::Closed(_)) => {
            counters.queued_bytes.fetch_sub(bytes, Ordering::Relaxed);
            counters.sink_failed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Counts one record the queue could not take and tells the host log on the first occurrence.
fn reject_for_full_queue(origin: &RecordOrigin, counters: &PluginLogCounters) {
    counters.queue_rejected.fetch_add(1, Ordering::Relaxed);
    if !counters.queue_full_reported.swap(true, Ordering::Relaxed) {
        counters.host_warnings.fetch_add(1, Ordering::Relaxed);
        ora_warn!(
            message = "plugin log queue is full; newest records are being dropped",
            plugin_id = %origin.plugin_id,
            generation = origin.generation,
        );
    }
}

/// Opens the production file sink and reports a recovered tail as a host fact, never its bytes.
fn open_file_sink(
    root: &Path,
    directory: &Path,
    plugin_id: &str,
) -> Result<PluginLogSink, SinkOpenError> {
    let sink = PluginLogSink::open(root, directory)?;
    if let Some(recovered) = sink.recovered_tail() {
        ora_warn!(
            message = "plugin log ended mid-record; the previous active file was preserved under a recovery name",
            plugin_id = %plugin_id,
            recovered_to = %recovered.display(),
        );
    }
    Ok(sink)
}

/// Drains the queue into the sink on a blocking thread until the reader closes it.
///
/// `open` runs first so a sink that cannot be opened — a path conflict, a writer of an earlier
/// generation or another host still holding the file, an I/O error — fails the whole generation
/// up front; the queue is still drained so the reader never blocks, and every dequeued record
/// is counted as lost.
pub(super) fn run_writer<S: LineSink>(
    mut queue: mpsc::Receiver<String>,
    open: impl FnOnce() -> Result<S, SinkOpenError>,
    plugin_id: &str,
    counters: &PluginLogCounters,
) {
    let mut sink = match open() {
        Ok(sink) => Some(sink),
        Err(error) => {
            report_sink_failure(plugin_id, counters, error.class(), &error.to_string());
            None
        }
    };
    let mut unflushed: u64 = 0;
    while let Some(line) = queue.blocking_recv() {
        counters
            .queued_bytes
            .fetch_sub(line.len(), Ordering::Relaxed);
        counters.dequeued.fetch_add(1, Ordering::Relaxed);
        let Some(open) = sink.as_mut() else {
            counters.sink_failed.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        match open.write_line(&line) {
            Ok(()) => unflushed += 1,
            Err(error) => {
                // This record and everything buffered before it may or may not have reached the
                // file; that is exactly the class the stats keep separate from "never written".
                counters
                    .indeterminate
                    .fetch_add(unflushed + 1, Ordering::Relaxed);
                unflushed = 0;
                report_sink_failure(plugin_id, counters, "write", &error.to_string());
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
            report_sink_failure(plugin_id, counters, "flush", &error.to_string());
            sink = None;
        }
    }
    if let Some(mut open) = sink
        && let Err(error) = open.flush()
    {
        counters
            .indeterminate
            .fetch_add(unflushed, Ordering::Relaxed);
        report_sink_failure(plugin_id, counters, "flush", &error.to_string());
    }
}

/// Records the sink as unavailable and tells the host log once per generation.
fn report_sink_failure(
    plugin_id: &str,
    counters: &PluginLogCounters,
    class: &'static str,
    error: &str,
) {
    if !counters.sink_failure_reported.swap(true, Ordering::Relaxed) {
        counters.host_warnings.fetch_add(1, Ordering::Relaxed);
        ora_warn!(
            message = "plugin log sink is unavailable for this generation",
            plugin_id = %plugin_id,
            generation = counters.generation.load(Ordering::Relaxed),
            class,
            error = %error,
        );
    }
}
