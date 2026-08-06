# ora-logging

`ora-logging` owns Ora's process-wide structured logging contract, local clock, sink composition, and shared emission helpers.

## Responsibilities

- `init_logging` installs the subscriber from an explicit `LoggingConfig`, initializes the immutable process timezone, and returns a `LoggingGuard` that keeps non-blocking file writers alive.
- File sinks use a lossy non-blocking writer so emission never blocks callers; `LoggingGuard::dropped_lines` exposes how many lines were discarded when the channel was full, and dropping the guard prints a one-line stderr summary when that count is non-zero.
- Output modes support stdout, daily rotating files, or both, with retention cleanup for matching file series.
- Events are formatted as one JSON object per line with stable top-level timestamp, level, target, message, method, span, trace, and request fields; business and error fields are grouped consistently.
- `ora_trace!`, `ora_debug!`, `ora_info!`, `ora_warn!`, and `ora_error!` attach the current method name and preserve the shared event shape.
- Correlation helpers create spans whose trace and request identifiers propagate into nested events.
- `ErrorReport::from_error` preserves the complete original `Error::source()` chain in debug builds
  and renders a bounded, single-line, redacted chain in release builds for the single
  request-completion event emitted by runtime seams.
- `clock` exposes local time and offsets from the IANA timezone fixed during startup.

## Boundaries

Initialization is process-wide and the timezone can be set only once. Runtime composition roots must parse environment configuration, call initialization before clock access, and retain `LoggingGuard` for the process lifetime.

This crate does not decide business log messages, public error classification, field allowlists, or
read environment variables. Callers remain responsible for excluding sensitive structured fields
before the report's residual regex redaction. File rotation naming follows the underlying
appender's daily boundary, while event timestamps remain authoritative local timestamps.

Test helpers install a thread-scoped TRACE dispatcher so shared tracing callsite interest cannot make structured-log tests order-dependent.

See [Runtime Logging](../../docs/runtime-logging.md) for configuration and the JSON event contract.
