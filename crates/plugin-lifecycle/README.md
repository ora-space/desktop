# ora-plugin-lifecycle

`ora-plugin-lifecycle` owns backend-only orchestration for installed Ora plugins. It joins
filesystem discovery, durable eligibility, process-scoped runtime state, application
invalidations, and the process data plane (calls and notifications) behind one lifecycle
interface.

## Control plane

Only explicit scans rebuild the installed snapshot. Per-plugin actions operate on cached identity,
serialize changes for the same plugin, and allow unrelated plugins to progress independently.
Missing durable state means disabled, and only the first enable creates a durable row. Uninstall
publishes a stopped runtime before durable or filesystem cleanup so later cleanup failures never
leave a process reported as running; after removing the package it also removes the plugin's data
directory.
Each scan reapplies durable eligibility to every retained package, stops runtimes that durable state
no longer permits, and removes durable rows for packages missing from disk. To return one coherent
snapshot, a scan acquires cached plugin operation locks in stable identifier order and may therefore
wait for an in-flight launch or stop to finish. This intentionally favors reconciliation consistency
over a partially refreshed result.

Every state transition is mirrored into a per-plugin `tokio::sync::watch` channel, which is what
`ensure_running` waits on; the managed-state map is only ever written through the accessors that
keep the two in sync.

## Launch

Before launching, the lifecycle creates `<data-dir>/plugin-data/<plugin_id>/` (with `downloads/`)
through `PluginDataDirectories`, derives Deno permissions from the plugin kind
(`permissions_for`: ui plugins may read and write only their data directory; agent plugins keep the
broad historical set, also exported as `agent_permissions` for the backend's agent supervisor), and
passes the package root as working directory plus `ORA_PLUGIN_DATA_DIR` in the environment. A
permission path containing a comma refuses to launch, because Deno reads commas as list separators.

After a successful handshake the registration is validated against the manifest kind
(`validate_registration`): a ui plugin with any remote-site surface must serve
`ui/downloadCompleted`; otherwise the runtime is stopped and the plugin enters `Failed`. Agent
contracts are verified by the backend's agent runtime, not here.

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
The production adapter launches Deno through the shared process-tree supervisor and waits for
confirmed process exit before filesystem cleanup.

Transport adapters and concrete dependency composition belong to `ora-backend` and Desktop. This
crate does not depend on Tauri, SQLite, or backend-private state.
