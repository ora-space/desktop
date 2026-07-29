# ora-logging

`ora-logging` owns Ora's process-wide structured logging contract, local clock, sink composition, and shared emission helpers.

## Responsibilities

- `init_logging` installs the subscriber from an explicit `LoggingConfig`, initializes the immutable process timezone, and returns a `LoggingGuard` that keeps non-blocking file writers alive.
- Output modes support stdout, daily rotating files, or both. File dates and rollover boundaries use the configured process timezone, and retention cleanup runs at startup and after each rollover.
- Events are formatted as one JSON object per line with stable top-level timestamp, level, target, message, method, span, trace, and request fields; business and error fields are grouped consistently.
- `ora_trace!`, `ora_debug!`, `ora_info!`, `ora_warn!`, and `ora_error!` attach the current method name and preserve the shared event shape.
- Correlation helpers create spans whose trace and request identifiers propagate into nested events.
- `clock` exposes local time and offsets from the IANA timezone fixed during startup.

## Boundaries

Initialization is process-wide and the timezone can be set only once. Runtime composition roots must parse environment configuration, call initialization before clock access, and retain `LoggingGuard` for the process lifetime.

This crate does not decide business log messages or read environment variables. File rotation and event timestamps share the explicit timezone supplied by the runtime composition root.

Test helpers install a thread-scoped TRACE dispatcher so shared tracing callsite interest cannot make structured-log tests order-dependent.

See [Runtime Logging](../../docs/runtime-logging.md) for configuration and the JSON event contract.
