# Runtime Logging

Ora Rust services initialize shared structured logging through `ora-logging`.

## Ownership boundary

- `ora-logging` owns the process-wide subscriber setup, JSON event formatting, sink selection, file rotation, retention cleanup, and the immutable process timezone.
- Runtime composition roots own reading configuration, calling `ora_logging::init_logging` with an explicit `LoggingConfig`, and retaining the returned `LoggingGuard` for the rest of the process lifetime. The guard keeps the non-blocking file writer alive; dropping it early loses buffered output.
- Runtime crates such as `ora-application`, `ora-db`, and `ora-backend` emit structured `tracing` events but never configure sinks or read environment variables.

Initialization is process-wide and the timezone can be set only once, so it must happen before any `ora_logging::clock` access. If a file sink cannot be created or prepared, initialization fails with a typed `LoggingInitError` instead of silently degrading to another sink.

## Web server configuration

`apps/web/server` maps these environment variables into `ora-logging`:

- `ORA_LOG_LEVEL`: `trace`, `debug`, `info`, `warn`, or `error`. Default: `info`. An unrecognized value fails startup.
- `ORA_LOG_MODE`: `stdout`, `file`, or `stdout_and_file`. Default: `stdout`. An unrecognized value fails startup.
- `ORA_LOG_MAX_DAYS`: retention window in days for file-backed logging, counting the current active file. Default: `3`. A non-numeric or zero value fails startup.
- `ORA_TIMEZONE`: IANA timezone used by structured event timestamps, such as `Asia/Shanghai` or `Europe/London`.

The log file path is **not** independently configurable. It is derived from the runtime data root as `<ORA_DATA_DIR>/logs/ora.log`, alongside the SQLite database and worktree root. See [Web Server Runtime](web-server-runtime.md).

`ORA_LOG_MODE=stdout` writes JSON lines to standard output only — no files are created and retention cleanup does not run. File-backed modes rotate daily and delete the oldest matching files first once the retained daily window would exceed `ORA_LOG_MAX_DAYS`. `stdout_and_file` emits every event to both sinks using the same envelope.

The Web server resolves its process timezone once during startup. A non-empty `ORA_TIMEZONE` takes precedence over the generic `TZ` environment variable. If neither is configured, startup warns and uses `Asia/Shanghai`. If the selected value is not a valid IANA timezone, startup warns and uses UTC without trying a lower-priority source. Values are trimmed before parsing.

## Desktop configuration

Desktop does not read logging environment variables. It builds its `LoggingConfig` in code: the file sink is `app_data_dir/logs/ora.log` with daily rotation and three retained days, debug builds write to stdout and the file while release builds write to the file only, and the timezone comes from the operating system. See [Desktop Runtime](desktop-runtime.md).

## JSON event contract

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

Business metadata belongs under `context`, and failure details belong under `error`. Field routing is by prefix: a field named `error.kind` lands in the `error` object, `context.operation` lands in `context`, and any other unrecognized field falls through into `context` so a plain `operation = "create_project"` is grouped correctly without ceremony. `context` and `error` are omitted entirely when empty.

```json
{
  "timestamp": "2026-05-09T20:00:00+08:00",
  "level": "INFO",
  "target": "ora_application::project::handlers",
  "message": "project operation completed",
  "method": "handle",
  "context": {
    "operation": "create_project",
    "project_id": "project-42"
  }
}
```

The RFC 3339 timestamp uses the configured process timezone and includes its UTC offset. The `tracing-appender` file writer still names and rotates daily files at UTC boundaries; event timestamps remain authoritative when a local calendar date differs from the file suffix.

## Emission helpers

Prefer `ora_logging::ora_trace!`, `ora_debug!`, `ora_info!`, `ora_warn!`, and `ora_error!` over the raw `tracing` macros. They attach the current function name as the top-level `method` field and preserve the shared event shape.

Correlation helpers — `runtime_span`, `span_with_correlation`, `span_with_trace_id`, `span_with_request_id` — create spans whose `span`, `trace_id`, and `request_id` propagate into nested events, so those reserved fields stay consistent without each call site repeating them. Explicit event fields still win over the enclosing span's values.

`ora_logging::clock` exposes local time and UTC offsets from the timezone fixed during startup. Runtime code should use `ora_logging::clock::now_local` rather than `OffsetDateTime::now_local()`.

## Git command logging

`gitlancer` stays framework-neutral: it defines a `GitlancerLogger` trait and a `logging::register` function backed by a write-once `OnceLock`. After the first registration every read is lock-free, a second `register` call is a no-op that keeps the first logger, and with no logger registered all call sites compile and run with zero side effects.

`CliGitRunner::run` calls `log_command` with `GitCommand::cwd` — the directory the git process will be spawned in, not the host program's working directory — and the full command string, immediately before spawning. After the process exits it calls `log_result` with the elapsed milliseconds, the exit code, and a success flag. A spawn failure (`GitExecError::GitNotFound` or `GitExecError::SpawnFailed`) also reports through `log_result` with failure and a zero duration.

`ora-logging` supplies the bridge. `OraGitlancerLogger` implements the trait by forwarding `log_command` to an `ora_info!` event carrying `cwd` and `command`, and `log_result` to `ora_info!` on success or `ora_error!` on failure, carrying `duration_ms` and `exit_code`. `register_gitlancer_logger()` constructs and registers it in one call.

Both runtime roots call `register_gitlancer_logger()` immediately after `init_logging`, so every Git command Ora runs is visible in the structured log.

## Testing

`with_trace_logging` and `with_recorded_trace_logging` install a thread-scoped `TRACE` dispatcher. Use them for tests that assert on structured output *and* for ordinary tests that merely touch the same callsites — `tracing` caches callsite interest, so an unscoped test running first can otherwise make a later log assertion fail intermittently.
