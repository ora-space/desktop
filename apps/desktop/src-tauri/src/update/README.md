# update

Desktop application self-update orchestration around Tauri's signed updater plugin.

## Responsibilities

- Own the Desktop-side lifecycle of software updates: when to check, when to download, when the
  user may install, and what the main webview is told about it.
- Hold a Desktop-scoped `ora_scheduler::Scheduler` that runs one delayed check after start and a
  recurring check afterwards.
- Persist a downloaded package in the identity-addressed `~/.ora/cache/desktop-updates/v2/`
  store, recover it after a process restart, and drop it once the running build supersedes it.
- Read the unsigned release notes a manifest may carry, apart from the check, so they are only ever
  shown or hidden and never change an update outcome.
- Decide whether the running installation is one the updater is allowed to replace in place.

## Non-responsibilities

- Update transport, manifest parsing, version comparison, and artifact selection belong to
  `tauri_plugin_updater`. The plugin verifies fresh downloads; recovery verifies persisted bytes
  again with `minisign-verify` and the same public key from Tauri configuration. The local SHA-256
  record detects corruption but is never treated as a signature or trust source.
- Release artifact production and `latest.json` generation belong to the build workflow.
- Proxy configuration is owned by the backend user configuration; this module only reads it.

## Boundaries

- `UpdateService` is the only public entry point besides the three Tauri commands re-exported from
  the module root (status, install, and an on-demand check). `DesktopUpdateStatus` and
  `ManualUpdateReason` are the wire contract shared with
  `packages/app-shell/src/platform/types.ts`.
- `DesktopUpdateMode` lets the composition root disable network work in development builds while
  keeping the commands and the state machine reachable from tests.

## Lifecycle

1. `UpdateService::start` opens the versioned artifact store, removes interrupted staging entries,
   discards releases the running build already includes, and — in `Enabled` mode — registers the
   delayed check and the cron job.
2. Each check rebuilds the updater so a proxy the user changed since the last check is picked up,
   then asks the plugin for an update.
3. A fresh manifest identity is matched against committed artifacts. Matching bytes are checked
   against their record and the fresh signature and reused; otherwise the plugin downloads and
   verifies a package.
4. A new package and record are prepared below `staging/` and the whole directory is renamed into
   `entries/<artifact-id>/`. Only after that commit does it replace an older ready artifact.
5. `install` re-reads and re-verifies the committed artifact immediately before handing the bytes
   to the plugin. Non-Windows platforms restart; Windows is terminated by the installer.

## Invariants

- Only one update operation runs at a time. The delayed first check and the cron schedule can
  overlap in principle, so both go through the same asynchronous operation lock.
- Runtime status and installable data occupy one `RuntimeUpdateState`; `Ready` cannot exist without
  the matching installer handle, fresh manifest descriptor, and verified artifact reference.
- A package already ready in this process stays advertised across a failed replacement check.
  The old entry remains untouched until a replacement directory is completely committed.
- A package left by an earlier process is not trusted from its record alone. A fresh manifest must
  match its release, target, bundle kind, signature fingerprint, and trust-root fingerprint, after
  which the bytes are re-verified and reused without another download.
- Raw manifest values never form cache paths. Entry names are SHA-256 content addresses, payload
  names come from the trusted bundle enum, and incomplete work remains isolated under `staging/`.

## Failure semantics

- A failed check is logged and surfaced as `Failed` only when nothing was installable in the
  current process before it. A persisted candidate is retained for a later reconciliation retry.
- A plugin installation failure restores `Ready` and keeps the verified artifact so the user can
  retry. A package that fails the pre-install record, digest, or signature checks is removed and
  transitions to `Failed` instead of advertising a broken retry.
- An installation that the updater cannot perform — a `deb` or `rpm` package, or a bare executable
  on Linux — is reported as `ManualUpdate` before any download is spent, because the static
  manifest only advertises an AppImage for Linux.

## Interactions

- `crate::state::DesktopState` holds the service; `crate::lib` builds it in the composition root.
- `ora_scheduler` provides the delayed and cron registrations.
- `ora_backend::Backend` provides the network proxy settings read before every check.
