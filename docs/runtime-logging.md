# Runtime Logging

Ora Rust services initialize shared structured logging through `ora-logging`.

## Ownership Boundary

- `ora-logging` owns the process-wide subscriber setup, JSON event formatting, sink selection, file rotation, and retention cleanup.
- Runtime composition roots such as `apps/web/server` own reading environment configuration, calling `ora_logging::init_logging`, and retaining the returned `LoggingGuard` for the rest of the process lifetime.
- Runtime crates such as `ora-application` and `ora-db` emit structured `tracing` events but do not configure sinks themselves.

## Environment Configuration

`apps/web/server` maps the following environment variables into `ora-logging`:

- `ORA_LOG_LEVEL`: `debug`, `info`, `warn`, or `error`. Default: `info`.
- `ORA_LOG_MODE`: `stdout`, `file`, or `stdout_and_file`. Default: `stdout`.
- `ORA_LOG_PATH`: base path for file-backed logging. Default: `./ora.log`.
- `ORA_LOG_MAX_DAYS`: retention window in days for file-backed logging, including the current active file. Default: `3`.
- `ORA_TIMEZONE`: IANA timezone used by structured event timestamps, such as `Asia/Shanghai`
  or `Europe/London`.

`ORA_LOG_MODE=stdout` ignores file path and retention settings. File-backed modes rotate daily and clean up older matching files once the retained daily window would exceed `ORA_LOG_MAX_DAYS`.

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

The RFC 3339 timestamp uses the configured process timezone and includes its UTC offset. The current
`tracing-appender` file writer still names and rotates daily files at UTC boundaries; event
timestamps remain authoritative when a local calendar date differs from the file suffix.

`ora-logging` also provides helper APIs for correlation-aware spans so runtime crates can attach `span`, `trace_id`, and `request_id` consistently.
For runtime event calls, prefer `ora_logging::ora_debug!`, `ora_logging::ora_info!`, `ora_logging::ora_warn!`, and `ora_logging::ora_error!`; these wrappers automatically attach the current function name as the top-level `method` field.
