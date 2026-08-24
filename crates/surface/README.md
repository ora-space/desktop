# Ora Surface

`ora-surface` is the Tauri-free domain layer for plugin-contributed UI surfaces: identity,
definitions, navigation policy, the instance state machine, the process-wide registry, and the
managed-download model. Everything here is pure Rust and covered by unit tests; the desktop host
executes the effects this crate decides on.

## Responsibilities

- `ids`: `SurfaceInstanceId` and `OperationId` (process-local counters), `ViewGeneration` (page
  rebuild counter), `DownloadId` (host-allocated download identity; a URL is not one), and
  `WebviewLabel` (`plugin-webview:<instance>` for workbench pages, `remote-webview:<instance>`
  for external sites, restricted to the Tauri label alphabet; the prefix follows the content
  source because host capabilities match on it). Labels are names only and never an
  authorization input.
- `definition`: `SurfaceDefinition` built from `ora-plugin-manager`'s already validated
  contribution, with `SurfaceSource::Workbench` (canonical asset root, entry document, declared
  bridge methods) or `SurfaceSource::RemoteSite` (start URL, navigation origins, download
  policy); `MountTarget` (`embedded` / `windowed` on the wire, both directions).
- `navigation`: `NavigationPolicy::RemoteSite` accepts only credential-free `https` URLs whose
  normalized origin is one of the declared origins; `NavigationPolicy::WorkbenchAssets` accepts
  only URLs below the instance's own asset base.
- `assets`: the `ora-plugin` scheme, per-platform asset base and entry URLs, `AssetRequest`
  (splits `/<instance>/<path>`), the servable content-type table, and the workbench CSP. Pure
  functions; the desktop protocol handler only adds file I/O.
- `state`: pure transition functions returning `SurfaceEffect`s (see `src/state/README.md` for
  the full transition table).
- `registry`: `SurfaceRegistry` keeps live instances behind one mutex, enforces the singleton
  instance policy, maps labels and instance ids to record snapshots (`resolve_label`, `record`)
  for authorization, pins workbench instances to one plugin process generation
  (`bind_workbench_generation`, first writer wins), and returns effects for lock-free execution.
- `downloads`: the frozen `DownloadIntent`, rule selection (`select_disposition`) against the
  page URL captured at request time, and the `ManagedDownload` state machine
  (`Staging → AwaitingChoice | Processing → Settled`), whose `choose` transition is the
  linearization point that keeps one download from running two actions.
- `events`: `SurfaceEvent`, the camelCase-tagged projection the desktop adapter emits verbatim.

## Non-responsibilities

- Creating, reparenting, destroying, or showing webviews; emitting events; file transfer;
  running download actions. Hosts do that by executing the returned `SurfaceEffect`s and
  driving the managed-download transitions with real outcomes.
- Manifest parsing and validation (`ora-plugin-manifest` / `ora-plugin-manager`).
- Resolving per-plugin data directories; callers pass paths explicitly.
- File-name sanitization and collision handling live in `ora-utils::fs`.

## Invariants

- Labels are never an authorization source; hosts resolve them through `SurfaceRegistry`.
- The registry lock only guards map updates and pure transitions; no I/O runs while it is held.
- A `SurfaceRecord` is a cloned snapshot; callers never hold references into the registry.
- `Closed` is not a state: an instance whose transition yields no next state is removed, along
  with its label mapping and generation binding.
- A workbench generation binding is written once per instance and never rebound; callers must
  compare the returned binding with the generation they are about to use.
- Completions must echo the `OperationId` of the operation that started them; a mismatch is a
  `StaleCompletion` and changes nothing.
- A managed download's disposition is frozen with its intent when the transfer starts; the page
  navigating away during the transfer changes nothing.
