//! Host-owned persistence of one plugin process's stderr diagnostics.
//!
//! stdout carries the Plugin Protocol and nothing else; every level of plugin diagnostics
//! travels over stderr, either as a versioned SDK envelope or as whatever bytes a dependency
//! wrote. The host reads that stream without ever blocking it, decodes it incrementally, stamps
//! the trusted identity itself, filters by the plugin's own persisted level, and appends JSONL
//! to `<data-dir>/plugins/logs/<namespace>/<name>/plugin.log` under an exclusive writer lock.
//! Host facts about the plugin — lifecycle, protocol, and the health of this very pipeline —
//! stay in the Ora runtime log; the plugin's own statements never do.

mod envelope;
mod pipeline;
mod record;
mod sink;

pub use envelope::{MAX_IDENTIFIER_BYTES, MAX_NESTING_DEPTH, PLUGIN_LOG_ENVELOPE_V1_PREFIX};
pub use pipeline::{
    MAX_QUEUE_BYTES, MAX_RECORD_BYTES, PluginLogSetup, PluginLogStats, PluginLogTeardown,
    QUEUE_CAPACITY,
};
pub use record::{DEFAULT_PLUGIN_TARGET, RAW_STDERR_TARGET, RESERVED_CONTEXT_KEYS};
pub use sink::{
    ACTIVE_LOG_FILE_NAME, SinkOpenError as PluginLogSinkError, WRITER_LOCK_FILE_NAME,
    confirm_writer_released as confirm_plugin_log_writer_released,
};

pub(crate) use pipeline::{PluginLogCounters, PluginLogPipeline, finish, start};

#[cfg(test)]
mod pipeline_tests;
