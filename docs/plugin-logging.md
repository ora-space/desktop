# Plugin Logging

English | [中文](plugin-logging.zh.md)

Each plugin process's diagnostics are persisted by the host into that plugin's own JSONL file,
separate from the Ora runtime log. The decision behind this design is
`specs/decisions/desktop/plugin/logging/0-host-owned-plugin-log.md`; its core test cases live under
`specs/test-cases/desktop/plugin/logging/`.

## Ownership boundary

- **stdout carries only the Plugin Protocol.** Nothing in the logging path writes to it.
- **stderr is the diagnostics transport for every level.** The SDK writes versioned envelopes; third-party
  libraries and native code may write anything. stderr does not mean `ERROR`.
- **The Ora runtime log records host facts** about a plugin: discovery, install, launch, exit, protocol and
  lifecycle failures, and the health of the plugin-log pipeline itself. Plugin statements never go there.
- **The plugin log records the plugin's own statements.** `ora-plugin-runtime` owns the pipeline
  (`crates/plugin-runtime/src/plugin_log.rs`); `ora-plugin-lifecycle` owns the per-plugin level
  (`crates/plugin-lifecycle/src/log_levels.rs`), the host session identity, and the storage boundary;
  `packages/plugin-sdk` owns the logger and the `console.*` mapping (`src/logger.ts`). The generic
  pieces — exclusive lock, no-follow open, line-tail probe, bounded line framing, lossless byte
  rendering — live in `ora-utils` (`crates/utils/src/fs`, `crates/utils/src/text`).

## Location and identity

```
<data-dir>/plugins/
  data/<namespace>/<name>/                    plugin data tree (the `ora/storage/*` root)
  logs/<namespace>/<name>/plugin.log          host-managed active log (JSONL)
  logs/<namespace>/<name>/plugin.log.lock     writer lock (sidecar, empty)
  logs/<namespace>/<name>/plugin.recovered-<stamp>[-n].log   preserved unterminated tails
  log-levels.json                             host-owned per-plugin levels
```

The log directory is a third persistent tree beside installed packages and plugin data, keyed by the
canonical Plugin ID, so every process generation and every installed version of the same identity
appends to the same file. The host derives the id from the installed package; nothing the plugin sends
chooses the path.

`ora/storage/*` resolves against the data tree, so no logical storage path can reach the logs tree and
no reserved name is needed; neither the Plugin Protocol nor the SDK offers a way to read a log. The
sink requires `plugins/logs` itself to be a plain directory and creates the tree below it level by
level: a level that already exists as a file, symlink, junction, or other reparse point is a conflict,
the sink refuses to open — it neither replaces nor deletes anything — and the generation runs with its
log unavailable while stderr is still drained. The active file is opened without following links and
re-checked through its handle.

### One writer per active file

Before touching `plugin.log`, a generation takes an exclusive advisory lock on the sidecar
`plugin.log.lock` (`ora_utils::fs::ExclusiveFileLock`, `File::try_lock`, never waiting). The lock is
held for the life of the writer and released when the writer thread finishes or its process exits,
so it excludes an earlier generation that has not released the file yet and another Ora host sharing
the same home alike. A generation that cannot take the lock runs with its sink unavailable: it keeps
draining stderr and counts every filtered-in record as `sink_failed`, but it never opens the file
beside the other writer. The lock is on a sidecar rather than on `plugin.log` because Windows range
locks would also block the read-only "Download log" copy while the plugin runs.

### Reopening after a partial write

A normal flush hands buffered lines to the operating system; it is not an `fsync`, and a crash or a
kill can leave the last line without its newline. Once the writer lock is held, the sink reads only
the file's final byte (`ora_utils::fs::classify_line_tail`). An empty or newline-terminated file is
appended to. A file that ends mid-line is renamed — never truncated, never overwritten — to
`plugin.recovered-<local timestamp>.log` (with `-1`, `-2`, … when that name is taken) in the same
directory, and a fresh `plugin.log` is created. The host logs the recovery as a fact without copying
any content; a recovery that fails (the rename is refused) makes the sink unavailable and leaves the
broken file exactly as found. Recovery files belong to the plugin's log directory and follow the
retain/delete disposition on uninstall. This isolates the tail only; it is not rotation.

## Transport format

The SDK writes one line per record: the prefix `@ora/plugin-log/v1 ` followed by a compact JSON
object with required `level` (`TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`) and `message`, and optional
`target`, `method`, `context` (object), and `error` (object). Newlines inside `message` are JSON-escaped,
so one record is always one stderr line. The stderr sink writes synchronously and continues a short
write until the whole envelope is out, so two records from the SDK never interleave.

`plugin.logger` exposes `trace`/`debug`/`info`/`warn`/`error` plus `child({ target, context })`. When
`run()` starts with the default transport it takes over exactly five console methods — before it
changes any state and before the first protocol frame — mapping `console.debug` to `DEBUG`,
`console.info` and `console.log` to `INFO`, `console.warn` to `WARN`, and `console.error` to `ERROR`,
all through the same logger. The takeover is never removed: it outlives initialization failures,
`ora/shutdown`, and the end of every callback, and later callers of the global methods (third-party
dependencies included) go through it too. If the console cannot be taken over (its methods are
frozen), `run()` throws before writing anything and the plugin never enters protocol operation. Output
before `run()` — module evaluation, `createPlugin`, `registerMethod` bodies — is the runtime's own
behavior; a method captured before the takeover, other workers or processes, other console methods,
and direct stdout writes are outside the guarantee. Errors, circular references, `BigInt`, throwing
getters, and oversized values degrade to bounded descriptions; logging never throws into plugin code
and never touches stdout.

The host decodes stderr incrementally, so records do not depend on pipe read boundaries. Anything that
is not a valid v1 envelope — plain text, the legacy `[plugin:<level>]` prefix, JSON without the prefix,
an unknown version, malformed JSON, or a payload that fails validation (wrong types, an identifier
over 256 bytes, `context` or `error` nested deeper than 16 levels) — is persisted as a raw `INFO`
record with `target = "plugin.stderr"`; invalid envelopes additionally carry `context.format_failure`.
Records above 64 KiB are split into raw fragments identified by `context.fragment`
(`sequence`, `index`, `last`); invalid UTF-8 is escaped reversibly (`\xNN`, backslashes doubled) and
marked with `context.encoding = "escaped-bytes"`.

## Persisted record

```json
{
  "timestamp": "2026-09-14T10:00:00+08:00",
  "level": "WARN",
  "target": "db",
  "message": "slow query",
  "method": "query",
  "context": {
    "ms": 12,
    "plugin_id": "official/example",
    "host_session_id": "5f1c6b0e-…",
    "generation": 3
  }
}
```

`timestamp`, `level`, `target`, and `message` are always present; `method`, `context`, and `error`
appear when supplied. `timestamp` is the host's receipt time from `ora_logging::clock::now_local`,
RFC 3339 with the local offset — not the plugin's event time, and not the disk write time; records
of one generation are persisted in receipt order even if the wall clock steps backwards. The host
writes `context.plugin_id`, `context.host_session_id`, and `context.generation` last: the session id
is minted once per host start and never reused, and the generation counts launches of the plugin
within that session, so two host runs that both launch the plugin as generation 1 stay
distinguishable. Before stamping, the host removes every reserved key the plugin supplied under
`context` — `plugin_id`, `generation`, `host_session_id`, `request_id`, `trace_id`, `span`,
`encoding`, `fragment`, `format_failure` — so neither identity, correlation, nor the host's own
decoding markers can be forged. Everything else in a record, `level`, `target`, `method`, `context`,
and `error` included, remains the plugin's own claim. Structured records without a `target` use
`plugin`.

## Per-plugin level

Every canonical Plugin ID has its own persisted level, default `INFO`, independent of the Ora runtime
log level and of every other plugin. The level is a floor applied by the host after decoding: records
at or above it keep their own level; raw text counts as `INFO`, so `WARN` or `ERROR` filters it by
policy without counting it as loss.

`getPluginLogLevel` / `setPluginLogLevel` read and change it. In the desktop app the control lives in
Settings → Plugins → Manage → row menu → Log level, shown only while developer mode is on; the same
menu offers "Download log", which copies the plugin's active `plugin.log` through the native save
dialog (`download_plugin_log`) without exposing its path. A change is persisted first and then
published to the running generation, so it applies to the next record without a restart; a failed
persist changes nothing and is reported as an error. Updates run under the plugin's operation lock,
the same lock uninstall holds, so they are ordered against it: an update that arrives after a
delete-data uninstall committed finds the identity uninstalled and is refused instead of recreating
the cleared setting. The setting survives upgrades and retain-data uninstalls; a delete-data
uninstall clears it (see Lifecycle).

## Backpressure and failure

The stderr reader never waits on the disk. It renders each accepted record to its JSON line and
offers it to a queue bounded both in count (1024 lines) and in bytes (8 MiB of rendered lines); a
maximal 64 KiB raw record of invalid bytes renders to at most eight times its size, so the byte bound
is what keeps memory finite. A full queue — by either measure — drops the **newest** records. A
failed sink (open, write, or flush) makes the rest of the generation count as lost while stderr keeps
being drained; the sink is never retried within a generation. Each generation counts `accepted`,
`queue_rejected`, `sink_failed`, `indeterminate` (handed to the file but undecidable after an I/O
error — the failed write and every line buffered since the last good flush), and `format_failures`,
queryable through `PluginRuntime::plugin_log_stats`. The Ora runtime log gets one `WARN` when the
queue first fills and one when the sink first fails (with the failure class: `path_conflict`,
`writer_busy`, `io`, `write`, `flush`), and a summary with counts when the generation ends — never one
line per lost record and never the plugin's payload. Log failures do not change the plugin's protocol
state and never terminate it.

## Lifecycle

Process exit is not log completion. After the process exits, the runtime runs the log teardown under
**one** five-second deadline shared by the stderr reader, the queue drain, and the flush: the reader
gets the first three quarters of it to reach EOF and is cut off after that, which closes the queue;
the writer gets whatever remains of the same deadline to drain, flush, and release the file. No stage
ever gets a fresh timeout. The teardown reports three things separately: whether stderr reached EOF
(if not, unread output of unknown size was abandoned — an inherited write end is the usual cause),
whether the writer released the file, and `queued_at_deadline`, the count of accepted records the
writer had not taken when time ran out. Only then does `shutdown_and_wait` (and therefore
`stop_plugin` and `uninstall_plugin`) return; the stop itself still succeeds.

A writer that misses the deadline is a blocking thread and cannot be aborted, and it is not
forgotten: it keeps the writer lock until it really finishes. A new generation may start meanwhile,
but it finds the sink busy, drains and counts, and does not write beside the old writer; a generation
started after the release persists normally. The same teardown runs on unexpected exit and on a
launch that never becomes ready. Host exit does not stop plugins through the lifecycle today — the
process reaper terminates survivors — so a plugin killed that way may leave an unterminated tail,
which the next generation's reopen sets aside as described above.

A delete-data uninstall first proves that no writer holds the plugin's log (it takes and releases the
writer lock; a busy lock fails the uninstall with `LogWriterActive` before anything moves), then
stages the package, the data tree, and the log tree in one same-volume transaction, then clears the
level setting while the trees are still only staged, and only then commits. If any move fails — on
Windows an open handle on `plugin.log` makes the log directory unrenamable — or the level clear
cannot be persisted, the earlier moves are rolled back and the uninstall reports failure with the
package, data, log, and level setting all intact and the plugin still installed. A retain-data
uninstall leaves the log tree and the level setting alone and needs no writer proof.

Host-managed child process output (`ora/childprocess/*`) is handed to the plugin and never persisted
automatically; the plugin forwards what it wants through its logger.
