# Plugin Logging

English | [中文](plugin-logging.zh.md)

Each plugin process's own diagnostics are persisted by the host into that plugin's JSONL log,
separate from Ora's runtime log. The decision is the
[host-owned plugin log](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/logging/0-host-owned-plugin-log.md)
ADR, with core test cases under
[`test-cases/desktop/plugin/logging/`](https://github.com/ora-space/specs/tree/main/test-cases/desktop/plugin/logging).

## Ownership

- **Stdout carries only the plugin protocol.** Nothing in the logging path writes to it.
- **Stderr is the diagnostics transport at every level.** The SDK writes versioned envelopes;
  third-party libraries and native code may write anything. Stderr does not mean `ERROR`.
- **Ora's runtime log records host facts** about a plugin: discovery, install, launch, exit,
  protocol and lifecycle failures, and the health of the plugin-log pipeline. Plugin statements never
  go there.
- **The plugin log records the plugin's own statements.** `ora-plugin-runtime` owns the pipeline
  (`src/plugin_log.rs`); `ora-plugin-lifecycle` owns per-plugin levels (`src/log_levels.rs`), the host
  session identity and the storage boundary; `packages/plugin-sdk` owns the logger and the `console.*`
  mapping (`src/logger.ts`). Generic pieces (exclusive lock, no-follow open, line-tail probe, bounded
  framing, lossless byte rendering) live in `ora-utils`.

## Location and identity

```text
~/.ora/plugins/
  data/<namespace>/<name>/                                 plugin data (the ora/storage root)
  logs/<namespace>/<name>/plugin.log                       active log (JSONL)
  logs/<namespace>/<name>/plugin.log.lock                  writer lock (empty sidecar)
  logs/<namespace>/<name>/plugin.recovered-<stamp>[-n].log preserved unterminated tails
  log-levels.json                                          per-plugin levels
```

The log tree is keyed by the canonical plugin id, so every process generation and every installed
version appends to the same file; nothing the plugin sends chooses the path. `ora/storage/*` resolves
against the data tree, so no storage path can reach the logs and neither the protocol nor the SDK can
read a log. The sink requires `plugins/logs` to be a plain directory and creates the levels below it;
an existing file, symlink, junction or reparse point at any level is a conflict, and the sink refuses
to open (replacing and deleting nothing) while stderr is still drained. The active file is opened
without following links.

**One writer.** Before touching `plugin.log`, a generation takes an exclusive, non-waiting lock on
`plugin.log.lock` for the writer's life. A generation that cannot take it runs with its sink
unavailable: it drains stderr and counts records as `sink_failed`, but never writes beside the other
writer. The lock is on a sidecar because Windows range locks on `plugin.log` would block the
read-only "Download log" copy.

**Reopening after a partial write.** A flush is not an `fsync`, so a crash can leave the last line
without its newline. Once the lock is held, the sink reads only the final byte: an empty or
newline-terminated file is appended to; a file ending mid-line is renamed (never truncated) to
`plugin.recovered-<local timestamp>.log` and a fresh file is created. A failed rename makes the sink
unavailable and leaves the file as found.

## Transport format

The SDK writes one line per record: the prefix `@ora/plugin-log/v1 ` and a compact JSON object with
required `level` (`TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`) and `message`, and optional `target`,
`method`, `context` and `error` objects. Newlines in `message` are escaped and writes are completed in
full, so records never interleave.

`plugin.logger` exposes `trace`, `debug`, `info`, `warn`, `error` and `child({ target, context })`.
When `run()` starts with the default transport it takes over `console.debug` (`DEBUG`),
`console.info` and `console.log` (`INFO`), `console.warn` (`WARN`) and `console.error` (`ERROR`)
before any protocol frame, and never removes the takeover. If the console cannot be taken over,
`run()` throws before writing anything. Output before `run()`, methods captured earlier, other
workers or processes and direct stdout writes are outside the guarantee. Errors, cycles, `BigInt`,
throwing getters and oversized values degrade to bounded descriptions; logging never throws into
plugin code.

The host decodes stderr incrementally. Anything that is not a valid v1 envelope (plain text, the
legacy `[plugin:<level>]` prefix, unprefixed JSON, an unknown version, malformed JSON or a payload
failing validation) is persisted as a raw `INFO` record with `target = "plugin.stderr"`; invalid
envelopes also carry `context.format_failure`. Records above 64 KiB are split into fragments
(`context.fragment`), and invalid UTF-8 is escaped reversibly and marked
`context.encoding = "escaped-bytes"`.

## Persisted record

```json
{
  "timestamp": "2026-09-14T10:00:00+08:00",
  "level": "WARN",
  "target": "db",
  "message": "slow query",
  "method": "query",
  "context": { "ms": 12, "plugin_id": "official/example", "host_session_id": "5f1c6b0e-…", "generation": 3 }
}
```

`timestamp` is the host's receipt time in local time with offset, not the plugin's event time, and
records of one generation keep receipt order. The host writes `context.plugin_id`,
`context.host_session_id` (minted once per host start) and `context.generation` (the launch count in
that session) last, after removing any reserved key the plugin supplied (`plugin_id`, `generation`,
`host_session_id`, `request_id`, `trace_id`, `span`, `encoding`, `fragment`, `format_failure`), so
identity, correlation and decoding markers cannot be forged. Everything else remains the plugin's own
claim. Structured records without a `target` use `plugin`.

## Per-plugin level

Every plugin has its own persisted level, default `INFO`, independent of the runtime log level and of
other plugins. It is a floor applied after decoding; raw text counts as `INFO`. `getPluginLogLevel` and
`setPluginLogLevel` read and change it; in the app the control is in Settings → Plugins → Manage → row
menu → Log level, shown only with developer mode on, next to "Download log" (`download_plugin_log`,
which copies the active file through the save dialog). A change is persisted first and then applied
to the running generation without restart; a failed persist changes nothing. Updates take the
plugin's operation lock, so one arriving after a delete-data uninstall is refused. The level survives
upgrades and retain-data uninstalls.

## Backpressure and failure

The stderr reader never waits on disk. Each record is rendered and offered to a queue bounded at 1024
lines and 8 MiB; a full queue drops the newest records. A failed sink (open, write or flush) makes the
rest of the generation count as lost while stderr keeps draining, with no retry within that
generation. Each generation counts `accepted`, `queue_rejected`, `sink_failed`, `indeterminate` and
`format_failures` (`PluginRuntime::plugin_log_stats`). The runtime log gets one `WARN` when the queue
first fills, one when the sink first fails (with its class: `path_conflict`, `writer_busy`, `io`,
`write`, `flush`) and a summary when the generation ends, never one line per lost record. Logging
failures never change protocol state or terminate the plugin.

## Lifecycle

Process exit is not log completion. After exit, log teardown runs under one five-second deadline
shared by the stderr reader (cut off after three quarters of it), the queue drain and the flush. The
teardown reports whether stderr reached EOF, whether the writer released the file and how many
accepted records were still queued. Only then does stop or uninstall return; the stop itself still
succeeds. A writer that misses the deadline keeps the lock until it really finishes, so a new
generation meanwhile runs with its sink busy. Host exit does not stop plugins through the lifecycle;
the process reaper terminates survivors, and any unterminated tail is set aside on the next reopen.

A delete-data uninstall first proves no writer holds the log (a busy lock fails with
`LogWriterActive`), stages the package, data and log trees in one same-volume transaction, clears the
level setting while they are only staged, and then commits. Any failed move (on Windows an open
handle makes the log directory unrenamable) or a failed level clear rolls everything back and leaves
the plugin installed. A retain-data uninstall leaves the log tree and level alone.

Output of host-managed child processes (`ora/childprocess/*`) goes to the plugin and is never
persisted automatically; the plugin forwards what it wants through its logger.

If the level file cannot be read or contains unsupported identities/levels, ordinary edits are
blocked and the original file is preserved. Readable entries remain effective. The log-level menu
shows the protected file path and disables editing; dedicated errors distinguish read-only
protection from a filesystem write failure. Warnings identify unsupported entries without logging
their values. Repair the file and restart to restore editing.

An explicit delete-data uninstall may remove only that plugin's key from a readable JSON object,
preserving every other raw JSON value verbatim, including unsupported values. Invalid or unreadable
files cannot be safely pruned; uninstall reports the repair path and rolls back staged removals.
Before either edit, the host checks that the on-disk bytes still match its loaded snapshot. An
external repair or deletion blocks the write until restart, preserving the newer disk contents.
