# Runtime Logging

Ora Rust services initialize shared structured logging through `ora-logging`.

## Ownership boundary

- `ora-logging` owns the process-wide subscriber setup, JSON event formatting, sink selection, file rotation, retention cleanup, and the immutable process timezone.
- Runtime composition roots own reading configuration, calling `ora_logging::init_logging` with an explicit `LoggingConfig`, and retaining the returned `LoggingGuard` for the rest of the process lifetime. The guard keeps non-blocking writer workers alive for every active sink (stdout and/or file); dropping it early loses buffered output. On shutdown, `WorkerGuard` waits briefly for the background worker to drain and may still drop remaining buffered events if a sink is slow. Stdout and file sinks are non-lossy: a full writer channel exerts backpressure on callers instead of dropping lines, so log integrity is preferred over never blocking under extreme sink stalls.
- Runtime request seams and infrastructure crates emit structured `tracing` events but never configure sinks or read environment variables. Application handlers and repository adapters do not emit generic completion or propagation-only failure events.

Initialization is process-wide and the timezone can be set only once, so it must happen before any `ora_logging::clock` access. If a file sink cannot be created or prepared, initialization fails with a typed `LoggingInitError` instead of silently degrading to another sink.

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

Long-lived streams, including task workspace watching, mark the response as deferred and emit completion when the stream ends, reports its typed contract error, is cancelled by the caller, or loses its Desktop channel. A dropped channel completes as `cancelled` because the caller stopped listening rather than the backend failing, so every stream forms exactly one completion event no matter which path tears it down. A terminal frame claims its completion from the backend outcome before it is sent, so the recorded outcome describes how the backend stream ended even when that last frame never reaches the client.

Ora frontend requests receive a canonical UUID v4 at the Tauri or stream entry seam. The same identifier correlates the request span, public error payload or stream error frame, and completion event. Client-provided request identifiers are never canonical.

Each request records exactly one completion event with `operation`, `request_id`, `outcome`, and `duration_ms`. Failures additionally record the stable public `error.code` plus the build-appropriate `error.message`, `error.chain`, and `error.chain_depth` produced by `ErrorReport::from_error`. Internal errors use `ERROR`, conflicts use `WARN`, `InvalidRequest` / `NotFound` / `PayloadTooLarge` / `Unprocessable` use `INFO`, and cancellation uses `DEBUG`. Successful health and readiness checks also use `DEBUG`.

`RequestLifecycle` guarantees that closing record structurally rather than by convention: when its last handle is dropped without any explicit completion, it emits `outcome = "abandoned"` at `DEBUG`. That covers a seam that forgets to complete and a deferred stream future that is dropped when its transport disappears, so a request can never leave only an opening record. The distinct outcome keeps those cases greppable instead of hiding them inside `cancelled`.

`ErrorReport` traverses the Rust `Error::source()` chain. In debug builds it preserves the original chain text without redaction, single-line conversion, or rendered length and depth limits so local diagnostics retain available context. In release builds it removes control characters, limits individual nodes, total output, and rendered chain depth, marks truncation, and applies precompiled regular expressions to residual secret-like text. In both modes traversal itself stops after 1,024 source nodes so a malformed cyclic chain cannot block completion logging; `error.chain_depth` saturates at that safety limit when the chain continues. Callers must still provide only approved structured fields: absolute paths, full Git arguments or remotes, SQL values, prompts, environment values, credentials, and unbounded stderr are not valid release log fields.

Git worktree and branch cleanup never runs synchronously inside a request: failures after Git resources are provisioned hand off to the durable `git_cleanup` background worker (see [Task Worktrees](task-worktrees.md)) instead of rolling back inline, so a later cleanup outcome never touches the primary request's error or source chain. That worker's own failures are recorded as independent `operation = "git_cleanup"` events — decoupled from the originating request id and carrying a plain error string rather than `ErrorReport::from_error`'s bounded chain — with retry/backoff state until a job completes or is marked for manual attention. Bootstrap, migration, and state-transition events likewise remain independent lifecycle facts.

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

`with_trace_logging` and `with_recorded_trace_logging` install a thread-scoped `TRACE` dispatcher. Use them for tests that assert on structured output _and_ for ordinary tests that merely touch the same callsites — `tracing` caches callsite interest, so an unscoped test running first can otherwise make a later log assertion fail intermittently.
