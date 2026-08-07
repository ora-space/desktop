# Runtime Logging

Ora Rust services initialize shared structured logging through `ora-logging`.

## Ownership boundary

- `ora-logging` owns the process-wide subscriber setup, JSON event formatting, sink selection, file rotation, retention cleanup, and the immutable process timezone.
- Runtime composition roots own reading configuration, calling `ora_logging::init_logging` with an explicit `LoggingConfig`, and retaining the returned `LoggingGuard` for the rest of the process lifetime. The guard keeps non-blocking writer workers alive for every active sink (stdout and/or file); dropping it early loses buffered output. On shutdown, `WorkerGuard` waits briefly for the background worker to drain and may still drop remaining buffered events if a sink is slow. Stdout and file sinks are non-lossy: a full writer channel exerts backpressure on callers instead of dropping lines, so log integrity is preferred over never blocking under extreme sink stalls.
- Runtime request seams and infrastructure crates emit structured `tracing` events but never configure sinks or read environment variables. Application handlers and repository adapters do not emit generic completion or propagation-only failure events.

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
  "target": "ora_backend::request_lifecycle",
  "message": "request completed",
  "method": "complete_success",
  "request_id": "550e8400-e29b-41d4-a716-446655440000",
  "context": {
    "operation": "create_project",
    "outcome": "success",
    "duration_ms": 12
  }
}
```

The RFC 3339 timestamp uses the configured process timezone and includes its UTC offset. The `tracing-appender` file writer still names and rotates daily files at UTC boundaries; event timestamps remain authoritative when a local calendar date differs from the file suffix.

## Request completion and errors

Long-lived streams, including task workspace watching, mark the response as deferred and emit completion only when the stream ends or reports its typed contract error.

Ora frontend requests receive a canonical UUID v4 at the Web, Tauri, or stream entry seam. The same identifier correlates the request span, public error payload or stream error frame, and completion event; Web also returns it through `X-Request-Id`. Client-provided request identifiers are never canonical.

Each request records at most one completion event with `operation`, `request_id`, `outcome`, and `duration_ms`. Failures additionally record the stable public `error.code` plus the build-appropriate `error.message`, `error.chain`, and `error.chain_depth` produced by `ErrorReport::from_error`. Internal errors use `ERROR`, conflicts use `WARN`, `InvalidRequest` / `NotFound` / `PayloadTooLarge` / `Unprocessable` use `INFO`, and cancellation uses `DEBUG`. Successful health and readiness checks also use `DEBUG`.

`ErrorReport` traverses the Rust `Error::source()` chain. In debug builds it preserves the original chain text without redaction, single-line conversion, or rendered length and depth limits so local diagnostics retain available context. In release builds it removes control characters, limits individual nodes, total output, and rendered chain depth, marks truncation, and applies precompiled regular expressions to residual secret-like text. In both modes traversal itself stops after 1,024 source nodes so a malformed cyclic chain cannot block completion logging; `error.chain_depth` saturates at that safety limit when the chain continues. Callers must still provide only approved structured fields: absolute paths, full Git arguments or remotes, SQL values, prompts, environment values, credentials, and unbounded stderr are not valid release log fields.

If cleanup or rollback also fails, the primary error remains unchanged. The secondary failure is recorded as a separate operation with the same request id and its own bounded error report; it is never attached to the primary source chain. Bootstrap, migration, and state-transition events likewise remain independent lifecycle facts.

These changes do not alter the deployment boundary: local JSONL and Logdy remain local viewing tools, while OTLP, Loki, Tempo, and Grafana retain their existing production responsibilities.

## Emission helpers

Prefer `ora_logging::ora_trace!`, `ora_debug!`, `ora_info!`, `ora_warn!`, and `ora_error!` over the raw `tracing` macros. They attach the current function name as the top-level `method` field and preserve the shared event shape.

Correlation helpers — `runtime_span`, `span_with_correlation`, `span_with_trace_id`, `span_with_request_id` — create spans whose `span`, `trace_id`, and `request_id` propagate into nested events, so those reserved fields stay consistent without each call site repeating them. Explicit event fields still win over the enclosing span's values.

`ora_logging::clock` exposes local time and UTC offsets from the timezone fixed during startup. Runtime code should use `ora_logging::clock::now_local` rather than `OffsetDateTime::now_local()`.

## Git command logging

`gitlancer` stays framework-neutral: it defines a `GitlancerLogger` trait and a `logging::register` function backed by a write-once `OnceLock`. After the first registration every read is lock-free, a second `register` call is a no-op that keeps the first logger, and with no logger registered all call sites compile and run with zero side effects.

`CliGitRunner::run` calls `log_command` immediately before spawning and passes the process cwd plus the assembled command to the framework-neutral logger. After the process exits it calls `log_result` with the elapsed milliseconds, the exit code, and a success flag; bounded-output and pipe failures also produce a failed result event. A spawn failure (`GitExecError::GitNotFound` or `GitExecError::SpawnFailed`) reports through `log_result` with failure and a zero duration.

`ora-logging` supplies the bridge. `OraGitlancerLogger` projects `log_command` to an `ora_info!` event containing the alphabetic Git subcommand as `operation`. For `git config`, it additionally records the recognized action (`action`), scope (`scope`), and safe `user.*` key (`key`), so identity lookups are distinguishable without exposing values. It still drops absolute cwd and all other arguments, including commit messages, paths, remotes, and credentials. `log_result` emits `ora_info!` on success or `ora_error!` on failure, carrying only `duration_ms` and `exit_code`. `register_gitlancer_logger()` constructs and registers it in one call.

Both runtime roots call `register_gitlancer_logger()` immediately after `init_logging`, so every Git command Ora runs is visible in the structured log.

## Testing

`with_trace_logging` and `with_recorded_trace_logging` install a thread-scoped `TRACE` dispatcher. Use them for tests that assert on structured output *and* for ordinary tests that merely touch the same callsites — `tracing` caches callsite interest, so an unscoped test running first can otherwise make a later log assertion fail intermittently.
