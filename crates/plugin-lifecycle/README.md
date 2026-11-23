# ora-plugin-lifecycle

`ora-plugin-lifecycle` owns backend-only orchestration for installed Ora plugins. It joins
filesystem discovery, runtime state, application
invalidations, and the process data plane (calls and notifications) behind one lifecycle
interface.

## Control plane

The Deno launcher accepts an injected process spawner for both the plugin and its host-spawned
children. Stopping closes child spawn admission under the same mutex as dispatch and tracking,
kills and reaps every tracked child, and propagates cleanup failures. A retained cleanup task
continues when a caller drops its stop future; the returned result waits for that task.

This crate is the sole owner of plugin processes. Nothing else in Ora starts, stops, or reaps one,
which is what keeps the runtime state reported to the settings surface identical to the processes
that actually exist. Every installed plugin is available. Agent and workbench processes start on
demand and may be stopped without changing availability. Webview, skill, MCP, and Hook plugins have
no process and therefore remain in the `stopped` runtime state.

Consumers that need to speak a protocol over a plugin connect to it instead of launching it
(`ensure_running` / `connection`, see the data plane below). This is how the agent runtime reaches
an agent plugin: it holds a `PluginConnection` pinned to one generation and reads that generation's
notifications through the sink, while activate, stop, scan, and uninstall decide how long the
process lives.

Only explicit scans rebuild the installed snapshot. List responses additionally read the current
Plugin Configuration summary from immutable package declarations and plugin-global value files
under `<data-dir>/plugins/data/<namespace>/<name>/store.json`, so the settings list reflects
persisted completeness without a separate editor fetch. When the cached package root is no longer
a traversable directory, the summary stays `NotDeclared` rather than `configuration_load_failed`. Per-plugin actions operate on cached
identity, serialize changes for the same plugin, and allow unrelated plugins to progress
independently.
Plugins begin stopped whenever the lifecycle opens. A scan retains runtime state for a package
that remains installed and valid, while a newly discovered package becomes immediately available
and stopped. A package with an invalid Plugin Configuration declaration remains visible but cannot
start. Uninstall
closes surfaces, stops the process, then atomically stages the complete
`plugins/installed/<namespace>/<name>` tree and, when selected, `plugins/data/<namespace>/<name>`
before committing the removal. A staging failure attempts every staged move in reverse order and
reports any rollback failure; after commit, staging cleanup is independent and
empty namespace directories are pruned. Cleanup failures retain their staged paths in memory and
are retried by later scans without reversing the already committed uninstall.
Each scan retains runtime state for valid packages, stops runtimes whose packages were
removed or became invalid, and forgets removed packages. To return one coherent
snapshot, a scan acquires cached plugin operation locks in stable identifier order and may therefore
wait for an in-flight launch or stop to finish. This intentionally favors reconciliation consistency
over a partially refreshed result.

Every state transition is mirrored into a per-plugin `tokio::sync::watch` channel, which is what
`ensure_running` waits on; the managed-state map is only ever written through the accessors that
keep the two in sync.

## Launch

Plugins are identified by `ora_domain::PluginId` (`<namespace>/<name>`); request contracts carry
the canonical string, and a malformed id is reported as `PluginNotFound`. Installed packages live
in `<data-dir>/plugins/installed/<namespace>/<name>/<version>` (read-only, discovered by
`ora-plugin-manager`); uninstall stages that package tree and, when requested, the plugin's data
directory.

Before launching, the lifecycle creates `<data-dir>/plugins/data/<namespace>/<name>/` (with
`downloads/`) through `PluginDataDirectories`, derives Deno permissions from the plugin kind
(`permissions_for`: workbench plugins get no `--allow-*` flag at all; webview, skill, and MCP
plugins are never launched; agent plugins keep the broad historical set, also exported as
`agent_permissions` for the backend's agent supervisor, and narrowing it is out of scope here),
and passes the package root as working directory. No
environment variable is injected: a plugin learns nothing about host paths. A permission path
containing a comma refuses to launch, because Deno reads commas as list separators.

The Deno launcher binds a `PluginStorage` handler to the plugin's data directory and hands it to
the runtime as the `HostRequestHandler` for that process, so plugin identity is fixed by the
launch, never by request params.

After a successful handshake the registration is validated against the manifest kind
(`validate_registration`). Workbench registrations may expose well-formed methods but cannot
declare emitted notifications. Webview, skill, MCP, and Hook plugins cannot register because they
have no process. Agent contracts are verified by the backend's agent runtime, not here.

## Host request methods

Every plugin process launches with exactly one `HostRequestHandler` (`ora-plugin-runtime`'s
`launch` is monomorphic over it), so `DenoPluginRuntimeLauncher::launch` composes the two request
namespaces below into one small dispatcher (`PluginHostRequests` in `runtime.rs`) that routes by
method prefix.

### Storage

`PluginStorage` serves `ora/storage/list`, `read`, `write`, and `remove`, all taking a logical
`path` relative to the plugin's data directory (`""` is the directory itself for `list`):

| Method               | Params                   | Result                                      |
| -------------------- | ------------------------ | ------------------------------------------- |
| `ora/storage/list`   | `{ path }`               | `{ entries: [{ name, kind, size_bytes }] }` |
| `ora/storage/read`   | `{ path }`               | `{ bytes_base64 }`                          |
| `ora/storage/write`  | `{ path, bytes_base64 }` | `{}` (parents created, atomic replace)      |
| `ora/storage/remove` | `{ path }`               | `{}` (file or whole directory tree)         |

`kind` is `file` or `directory`; symlinks and special files are never listed. Paths are parsed
with `ora-utils::path::PortableRelativePath` (no absolute paths, `..`, or NUL), resolved under a
`CanonicalPathRoot`, and must be reached without traversing any symlink; the host-owned
`web-profile/` subtree is refused at the first segment and hidden from root listings. Files are
capped at `MAX_STORAGE_FILE_BYTES` (8 MiB) in both directions because base64 must fit the 16 MiB
frame. Failures are JSON-RPC errors whose `data.kind` is one of `invalid_params` (`-32602`),
`invalid_path` (`-32602`), `not_found` (`-32004`), `too_large` (`-32005`), or `io` (`-32000`);
unknown `ora/storage/*` methods get `-32601`. Filesystem work runs on the blocking pool.

### Child processes

`PluginProcessHost` lets a plugin ask the host to own a subprocess on its behalf instead of
spawning one itself inside its own sandboxed runtime — the process is created and torn down
through `ora-process`'s tree-wide termination (a Windows Job Object or a Unix process group), the
same guarantee every other Ora-managed child process already gets. Only agent plugins can reach
it: `DenoPluginRuntimeLauncher` mounts `PluginProcessHost` per launch based on
`PluginLaunchRequest::allow_childprocess`, which is set only for the agent contribution kind. A
workbench, webview, skill, or MCP plugin runs with zero Deno permissions specifically so it cannot
start a process (see `permissions::permissions_for`); every `ora/childprocess/*` call from one of
those kinds gets the same `-32601` a genuinely unknown method would.

| Method                         | Params                                             | Result                                   |
| ------------------------------ | -------------------------------------------------- | ---------------------------------------- |
| `ora/childprocess/spawn`       | `{ command \| packageCommand, args?, cwd?, env? }` | `{ processId, pid }`                     |
| `ora/childprocess/write`       | `{ processId, bytesBase64 }`                       | `{}`                                     |
| `ora/childprocess/close_stdin` | `{ processId }`                                    | `{}` (signals EOF, does not kill)        |
| `ora/childprocess/kill`        | `{ processId }`                                    | `{}` (idempotent, best-effort tree-kill) |

A spawn request names its executable exactly one of two ways, and the choice decides who resolves
it. `command` is passed to the operating system unchanged — a PATH lookup, or a path the plugin
already knows. `packageCommand` is a package-relative path the host joins onto that plugin's own
install root, canonicalizes, and requires to be a regular file still inside the package; that is
how a plugin runs an executable it ships. The two are mutually exclusive rather than one falling
back to the other, because a relative `command` and a `packageCommand` are indistinguishable as
strings. Resolving package paths here rather than in the plugin is what keeps the "a plugin learns
nothing about host paths" property intact while still letting one ship a binary: the plugin is told
no host path and needs none, and `cwd` stays free to be the workspace the child runs in instead of
doubling as the directory its program is resolved against (which a relative program does
differently per platform).

One plugin source is published either as a package that bundles its CLI or as one that expects the
user's own install to be found on PATH, and it cannot know at build time which package it ended up
in. The host answers that at spawn time: a `packageCommand` the package does not carry at all is
reported as `package_command_missing`, apart from every other resolution failure, so the plugin can
fall back to `command` on exactly that condition and treat the rest as the package fault it is. The
SDK's `spawnAgentProcess` is that ladder.

The host pushes back three notifications the plugin never declares receiving (mirroring how
`agent/acp` already flows host→plugin): `ora/childprocess/stdout` and `ora/childprocess/stderr`
(`{ processId, bytesBase64 }`, raw chunks — the plugin owns any line framing) and
`ora/childprocess/exit` (`{ processId, code, signal }`, `signal` only ever set on Unix).
`processId` is scoped to one plugin generation, not globally unique. Failures carry `data.kind`
`invalid_params` / `invalid_command` / `package_command_missing` / `invalid_package_command`
(`-32602`), `not_found` (`-32004`), `program_not_found` or `io` (`-32000`) — `program_not_found`
means the OS could not resolve the executable, distinct from any other spawn or I/O failure.
`package_command_missing` means the package carries nothing at that path, which is how a package
built without a bundled CLI answers, and is the one failure a plugin should fall back to a PATH
lookup on. `invalid_package_command` is the answer for a `packageCommand` that is not a portable
relative path, escapes the package, or is not a regular file: such a package fails identically on
every retry, so it must not be mistaken for a machine that is merely missing a CLI. `write` rejects
a `bytesBase64` payload that would decode to more than `MAX_WRITE_BYTES` (8 MiB, mirroring
`MAX_STORAGE_FILE_BYTES`) as `invalid_params` before decoding it, so a plugin cannot force
unbounded host memory growth through one oversized write.

The exit notification for one process is never sent ahead of its last `stdout`/`stderr` chunk: the
exit watcher joins that process's output-pumping tasks before pushing `ora/childprocess/exit`,
even though the process's `wait()` and its pipes are read by independent tasks that would otherwise
race.

Every process a plugin generation spawned this way is killed, best effort, the moment that
generation's `PluginRuntime` reports exit for any reason — intentional stop, uninstall, restart,
or failure (`PluginProcessHost::kill_all`, awaited off `wait_for_exit` in
`DenoPluginRuntimeLauncher::launch`) — so a host-spawned process never outlives the plugin that
asked for it.

## Data plane

`PluginRuntime` exposes `registration`, `invoke`, and `notify` in addition to stop and exit
observation. `PluginLifecycle::connection` returns a `PluginConnection` pinned to one
`PluginGeneration` (the launch attempt) for a running plugin; `ensure_running` activates a stopped
or failed plugin on demand and waits, bounded, for it to run. Callers hold a connection only for one
interaction so a restarted plugin is never addressed through a stale generation.

Each launch spawns two background tasks guarded by the same attempt: an exit monitor that records
stop or failure, and a notification pump that forwards plugin-originated notifications to the
injected `PluginNotificationSink` as `InboundNotification`s. If the notification stream closes
while the process is still alive past a short grace period, the pump marks the attempt failed
("plugin notification channel closed"); whichever task transitions first wins and the other
observes the attempt mismatch and returns.

## Surface closing

`SurfaceCloser` is installed after construction via `set_surface_closer` because surfaces belong to
the desktop shell, which exists only after the backend is built. Stop and uninstall call
it inside the plugin's operation lock before stopping the runtime, so "uninstall while a surface is
open" needs no coordination beyond that lock. Until a closer is installed, closing is a no-op.

## Boundaries

Filesystem package parsing remains in `ora-plugin-manager`, while process protocol ownership remains
in `ora-plugin-runtime`.
The production adapter launches Deno through the shared process-tree supervisor with the sandbox
permissions the contribution kind requires, and waits for confirmed process exit before filesystem
cleanup. Startup discovery also reports every package that was skipped, because a package that
never became a plugin is otherwise invisible to an operator.

Transport adapters and concrete dependency composition belong to `ora-backend` and Desktop. This
crate does not depend on Tauri, SQLite, or backend-private state.
