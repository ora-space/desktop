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
  (`crates/plugin-lifecycle/src/log_levels.rs`) and the storage boundary; `packages/plugin-sdk` owns the
  logger and the `console.*` mapping (`src/logger.ts`).

## Location and identity

```
<data-dir>/plugins/
  data/<namespace>/<name>/           plugin data tree (the `ora/storage/*` root)
  logs/<namespace>/<name>/plugin.log host-managed active log (JSONL)
  log-levels.json                    host-owned per-plugin levels
```

The log directory is a third persistent tree beside installed packages and plugin data, keyed by the
canonical Plugin ID, so every process generation and every installed version of the same identity
appends to the same file. The host derives the id from the installed package; nothing the plugin sends
chooses the path.

`ora/storage/*` resolves against the data tree, so no logical storage path can reach the logs tree and
no reserved name is needed; neither the Plugin Protocol nor the SDK offers a way to read a log. The
sink creates the tree level by level under the canonical logs root: a level that already exists as a
file, symlink, or reparse point is a conflict, the sink refuses to open — it neither replaces nor
deletes anything — and the generation runs with its log unavailable while stderr is still drained.
The active file is opened without following links and re-checked through its handle.

## Transport format

The SDK writes one line per record: the prefix `@ora/plugin-log/v1 ` followed by a compact JSON
object with required `level` (`TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`) and `message`, and optional
`target`, `method`, `context` (object), and `error` (object). Newlines inside `message` are JSON-escaped,
so one record is always one stderr line.

`plugin.logger` exposes `trace`/`debug`/`info`/`warn`/`error` plus `child({ target, context })`. Once
`run()` starts with the default transport, `console.debug` maps to `DEBUG`, `console.info` and
`console.log` to `INFO`, `console.warn` to `WARN`, and `console.error` to `ERROR`, all through the same
logger. Errors, circular references, `BigInt`, throwing getters, and oversized values degrade to bounded
descriptions; logging never throws into plugin code and never touches stdout.

The host decodes stderr incrementally, so records do not depend on pipe read boundaries. Anything that
is not a valid v1 envelope — plain text, the legacy `[plugin:<level>]` prefix, JSON without the prefix,
an unknown version, malformed JSON, or a payload that fails validation — is persisted as a raw `INFO`
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
  "context": { "ms": 12, "plugin_id": "official/example", "generation": 3 }
}
```

`timestamp`, `level`, `target`, and `message` are always present; `method`, `context`, and `error`
appear when supplied. The host stamps `timestamp` at receipt and writes `context.plugin_id` and
`context.generation` last, overwriting anything the plugin supplied under those keys. `request_id`,
`trace_id`, and `span` are never promoted to top-level fields from plugin input. Structured records
without a `target` use `plugin`.

## Per-plugin level

Every canonical Plugin ID has its own persisted level, default `INFO`, independent of the Ora runtime
log level and of every other plugin. The level is a floor applied by the host after decoding: records
at or above it keep their own level; raw text counts as `INFO`, so `WARN` or `ERROR` filters it by
policy without counting it as loss.

`getPluginLogLevel` / `setPluginLogLevel` read and change it. In the desktop app the control lives in
Settings → Plugins → Manage → row menu → Log level, shown only while developer mode is on; the same
menu offers "Download log", which copies the plugin's active `plugin.log` through the native save
dialog (`download_plugin_log`) without exposing its path. A change is persisted first and then published to the running generation, so it applies to
the next record without a restart; a failed persist changes nothing and is reported as an error. The
setting survives upgrades and retain-data uninstalls; a delete-data uninstall clears it, and a failed
clear fails the uninstall instead of reporting a complete cleanup.

## Backpressure and failure

The stderr reader never waits on the disk: records pass through a bounded queue (1024) and a blocking
writer. A full queue drops the **newest** records; a failed sink (open or write) makes the rest of the
generation count as lost while stderr keeps being drained. Each generation counts `queue_rejected`,
`sink_failed`, `indeterminate` (handed to the file but undecidable after an I/O error), and
`format_failures`, queryable through `PluginRuntime::plugin_log_stats`. The Ora runtime log gets one
`WARN` when the queue first fills and one when the sink first fails, and a summary with counts when the
generation ends — never one line per lost record and never the plugin's payload. Log failures do not
change the plugin's protocol state and never terminate it.

## Lifecycle

Process exit is not log completion. After the process exits, the runtime waits up to five seconds for
stderr to reach EOF, then closes the queue and waits for the writer to flush and release the file. Only
then does `shutdown_and_wait` (and therefore `stop_plugin` and `uninstall_plugin`) return, so a new
generation never writes beside the old one. If EOF never arrives (an inherited write end) or the flush
fails, the deadline ends the generation and the remainder is reported as unknown; the stop itself
still succeeds.

A delete-data uninstall stages the package, the data tree, and the log tree in one same-volume
transaction after the stop has released the log file: if any move fails — on Windows an open handle on
`plugin.log` makes the log directory unrenamable — the earlier moves are rolled back and the uninstall
reports failure with package, data, log, and level setting all intact. A retain-data uninstall leaves
the log tree and the level setting alone.

Host-managed child process output (`ora/childprocess/*`) is handed to the plugin and never persisted
automatically; the plugin forwards what it wants through its logger.
