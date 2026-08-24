# ora-plugin-lifecycle

`ora-plugin-lifecycle` owns backend-only orchestration for installed Ora plugins. It joins
filesystem discovery, durable eligibility, process-scoped runtime state, application
invalidations, and the process data plane (calls and notifications) behind one lifecycle
interface.

## Control plane

This crate is the sole owner of plugin processes. Nothing else in Ora starts, stops, or reaps one,
which is what keeps the runtime state reported to the settings surface identical to the processes
that actually exist. When a process starts depends on the contribution kind: enabling an agent
plugin also launches it, because its agent supervisor attaches to a running process rather than
starting one, so durable intent and reported runtime never disagree beyond the transition itself;
enabling a ui plugin only records eligibility, because its process is started on demand by the
first surface that needs it and stopped again when it has been idle.

Consumers that need to speak a protocol over a plugin connect to it instead of launching it
(`ensure_running` / `connection`, see the data plane below). This is how the agent runtime reaches
an agent plugin: it holds a `PluginConnection` pinned to one generation and reads that generation's
notifications through the sink, while enable, stop, scan, and uninstall keep deciding how long the
process lives.

Only explicit scans rebuild the installed snapshot. Per-plugin actions operate on cached identity,
serialize changes for the same plugin, and allow unrelated plugins to progress independently.
Missing durable state means disabled, and only the first enable creates a durable row. Uninstall
publishes a stopped runtime before durable or filesystem cleanup so later cleanup failures never
leave a process reported as running. Filesystem cleanup removes the complete
`plugins/installed/<namespace>/<name>` tree so no older version can reappear after the selected
version is uninstalled, prunes the namespace directory when it is empty, and then removes the
plugin's data directory.
Each scan reapplies durable eligibility to every retained package, stops runtimes that durable state
no longer permits, and removes durable rows for packages missing from disk. To return one coherent
snapshot, a scan acquires cached plugin operation locks in stable identifier order and may therefore
wait for an in-flight launch or stop to finish. This intentionally favors reconciliation consistency
over a partially refreshed result.

Every state transition is mirrored into a per-plugin `tokio::sync::watch` channel, which is what
`ensure_running` waits on; the managed-state map is only ever written through the accessors that
keep the two in sync.

## Launch

Plugins are identified by `ora_domain::PluginId` (`<namespace>/<name>`); request contracts carry
the canonical string, and a malformed id is reported as `PluginNotFound`. Installed packages live
in `<data-dir>/plugins/installed/<name>/` (read-only, discovered by `ora-plugin-manager`);
uninstall removes that package directory and the plugin's data directory.

Before launching, the lifecycle creates `<data-dir>/plugins/data/<namespace>/<name>/` (with
`downloads/`) through `PluginDataDirectories`, derives Deno permissions from the plugin kind
(`permissions_for`: ui plugins get no `--allow-*` flag at all; agent plugins keep the broad
historical set, also exported as `agent_permissions` for the backend's agent supervisor, and
narrowing it is out of scope here), and passes the package root as working directory. No
environment variable is injected: a plugin learns nothing about host paths. A permission path
containing a comma refuses to launch, because Deno reads commas as list separators.

The Deno launcher binds a `PluginStorage` handler to the plugin's data directory and hands it to
the runtime as the `HostRequestHandler` for that process, so plugin identity is fixed by the
launch, never by request params.

After a successful handshake the registration is validated against the manifest kind
(`validate_registration`): a ui plugin with any remote-site surface must serve
`ora/ui/download_completed`; one with any panel surface must serve `ora/ui/request` and declare
`ora/ui/push` in `emits`; otherwise the runtime is stopped and the plugin enters `Failed`. Agent
contracts are verified by the backend's agent runtime, not here. The `ora/ui/*` method names are
defined once in `registration` and exported (`UI_*_METHOD`) for the desktop surface host.

## Storage host methods

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
the desktop shell, which exists only after the backend is built. Stop, disable, and uninstall call
it inside the plugin's operation lock before stopping the runtime, so "uninstall while a surface is
open" needs no coordination beyond that lock. Until a closer is installed, closing is a no-op.

## Boundaries

Filesystem package parsing remains in `ora-plugin-manager`, process protocol ownership remains in
`ora-plugin-runtime`, and durable eligibility remains behind the `ora-application` repository port.
The production adapter launches Deno through the shared process-tree supervisor with the sandbox
permissions the contribution kind requires, and waits for confirmed process exit before filesystem
cleanup. Startup discovery also reports every package that was skipped, because a package that
never became a plugin is otherwise invisible to an operator.

Transport adapters and concrete dependency composition belong to `ora-backend` and Desktop. This
crate does not depend on Tauri, SQLite, or backend-private state.
