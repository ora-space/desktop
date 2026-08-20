# Ora Surface

`ora-surface` is the Tauri-free domain layer for plugin-contributed UI surfaces: identity,
definitions, navigation policy, the instance state machine, the process-wide registry, and
download reservations. Everything here is pure Rust and covered by unit tests; the desktop host
executes the effects this crate decides on.

## Responsibilities

- `ids`: `SurfaceDefinitionId` (plugin id + surface id, stable across restarts),
  `SurfaceInstanceId` and `OperationId` (process-local counters), `ViewGeneration` (page rebuild
  counter), and `WebviewLabel` (`remote-surface:<plugin id with '.' -> '_'>:<surface>:<instance>`,
  restricted to the Tauri label alphabet).
- `definition`: `SurfaceDefinition` built from `ora-plugin-manager`'s already validated
  `InstalledSurface`; `MountTarget` (`embedded` / `windowed` on the wire).
- `navigation`: `NavigationPolicy::allows` accepts only credential-free, port-free `https` URLs
  whose host is an exact allow-list entry or a subdomain of a suffix entry.
- `state`: pure transition functions returning `SurfaceEffect`s (see `src/state/README.md` for
  the full transition table).
- `registry`: `SurfaceRegistry` keeps live instances behind one mutex, enforces the singleton
  instance policy, maps labels to records for authorization, and returns effects for lock-free
  execution.
- `downloads`: `DownloadCoordinator` reserves `.part` paths per `(label, url)`, promotes or
  removes them on finish, and reports `CompletedDownload` with size and local completion time.
- `events`: `SurfaceEvent`, the camelCase-tagged projection the desktop adapter emits verbatim.

## Non-responsibilities

- Creating, reparenting, destroying, or showing webviews; emitting events; file transfer. Hosts do
  that by executing the returned `SurfaceEffect`s and download decisions.
- Manifest parsing and validation (`ora-plugin-manager`).
- Resolving per-plugin data directories; callers pass the download directory explicitly.
- File-name sanitization and collision handling live in `ora-utils::fs`.

## Invariants

- Labels are never an authorization source; hosts resolve them through `SurfaceRegistry`.
- The registry lock only guards map updates and pure transitions; no I/O runs while it is held.
- A `SurfaceRecord` is a cloned snapshot; callers never hold references into the registry.
- `Closed` is not a state: an instance whose transition yields no next state is removed.
- Completions must echo the `OperationId` of the operation that started them; a mismatch is a
  `StaleCompletion` and changes nothing.
- Download names are sanitized, keep their extension, and never overwrite an existing file, even
  when one appears during the transfer.
