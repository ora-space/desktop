# Runtime Logging

Ora Rust services initialize shared structured logging through `ora-logging`.

## Ownership Boundary

- `ora-logging` owns the process-wide subscriber setup, JSON event formatting, sink selection, file rotation, and retention cleanup.
- Runtime composition roots such as `apps/web/server` own reading environment configuration, calling `ora_logging::init_logging`, retaining the returned `LoggingGuard` for the rest of the process lifetime, and exposing its cloneable health handle through backend diagnostics when needed.
- Runtime crates such as `ora-application` and `ora-db` emit structured `tracing` events but do not configure sinks themselves.

## Environment Configuration

`apps/web/server` maps the following environment variables into `ora-logging`:

- `ORA_LOG_LEVEL`: `debug`, `info`, `warn`, or `error`. Default: `info`.
- `ORA_LOG_MODE`: `stdout`, `file`, or `stdout_and_file`. Default: `stdout`.
- `ORA_LOG_PATH`: base path for file-backed logging. Default: `./ora.log`.
- `ORA_LOG_MAX_DAYS`: maximum number of dated files retained for file-backed logging, including the current active file. Default: `3`.
- `ORA_TIMEZONE`: IANA timezone used by structured event timestamps, daily filename dates, and
  rollover boundaries, such as `Asia/Shanghai` or `Europe/London`.

`ORA_LOG_MODE=stdout` ignores file path and retention settings. File-backed modes rotate when the
non-blocking worker processes its first write after midnight in the configured process timezone.
The filename suffix is the worker's local processing date. An event formatted before midnight but
processed after midnight can therefore appear in the next day's file while retaining its earlier
JSON timestamp. Startup cleans the oldest matching files synchronously. Each successful rollover
schedules coalesced cleanup on a dedicated retention worker so directory scans and deletions do not
block the non-blocking writer.

File output intentionally keeps the lossy `tracing_appender` queue so logging backpressure cannot
stall application threads. `LoggingGuard::health()` returns a `LoggingHealthSnapshot`, and
`LoggingGuard::health_handle()` returns a cloneable handle suitable for backend health services.
The snapshot reports active rotation, output-write, output-flush, and retention issues together
with cumulative failure counters and the number of events discarded after the queue reached
capacity.

The Web server resolves its process timezone once during startup. A non-empty `ORA_TIMEZONE` takes
precedence over the generic `TZ` environment variable. If neither is configured, startup warns and
uses `Asia/Shanghai`. If the selected value is not a valid IANA timezone, startup warns and uses UTC
without trying a lower-priority source. Values are trimmed before parsing.

## JSON Event Contract

Every `ora-logging` sink writes one JSON object per line with these top-level fields:

- `timestamp`
- `level`
- `target`
- `message`

Optional top-level fields are emitted only when runtime code attaches them:

- `method`
- `span`
- `trace_id`
- `request_id`

Business metadata belongs under `context`, and failure details belong under `error`. For example:

```json
{
  "timestamp": "2026-05-09T20:00:00+08:00",
  "level": "INFO",
  "target": "ora_application::project::handlers",
  "message": "project operation completed",
  "context": {
    "operation": "create_project",
    "project_id": "project-42"
  }
}
```

The RFC 3339 timestamp records the event formatting time in the configured process timezone and
includes its UTC offset. Daily log filename dates and rollover boundaries use that same timezone
but are selected later from the non-blocking worker's processing time. Files are therefore
processing-date buckets rather than strict partitions of event timestamp dates. If the next file
cannot be opened, logging continues in the previous file and retries with exponential backoff from
one second up to sixty seconds. Failure to create the log directory or open the initial file
rejects initialization. Failure to delete old files does not reject initialization or stop file
writes; it appears as an active retention issue until a later cleanup succeeds.

`ora-logging` also provides helper APIs for correlation-aware spans so runtime crates can attach `span`, `trace_id`, and `request_id` consistently.
For runtime event calls, prefer `ora_logging::ora_debug!`, `ora_logging::ora_info!`, `ora_logging::ora_warn!`, and `ora_logging::ora_error!`; these wrappers automatically attach the current function name as the top-level `method` field.
