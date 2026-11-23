# Desktop Host

English | [中文](desktop-host.zh.md)

`apps/desktop/src-tauri` (`ora-desktop`) is the native Tauri host. It bootstraps the shared
`ora-backend::Backend`, exposes commands to the frontend, owns native windows and dialogs, and
adapts operating-system capabilities: file-manager handoff, opening URLs in the system browser,
plugin surface webviews and self-update. It owns no domain persistence or agent semantics; those
stay in the backend and contract crates. Crate-level details are in
[`apps/desktop/src-tauri/README.md`](../../apps/desktop/src-tauri/README.md).

## Startup

1. `--help`/`-h` and `--version`/`-V` print and exit before any window, database or plugin runtime
   opens. Any other argument is a normal graphical launch.
2. The data root is `ORA_DATA_DIR` when set (relative values resolve against the cwd), otherwise
   Tauri's `app_data_dir` for the identifier `space.ora.desktop`.
3. Desktop takes an exclusive `instance.lock` in the data root without waiting and holds it until
   the process exits; the OS releases it on exit or crash. A second launch focuses the existing
   window and exits without opening the database, writing why it left to stdout or the parent
   console. Startup work that assumes no other live process, such as the workflow boot sweep, runs
   only after the lock is held.
4. Logging is initialized (see [Logging](../core/logging.md)), then the `ora-reaper` sidecar starts,
   then the Backend opens: SQLite bootstrap, Skill storage reconciliation, plugin discovery, agent
   supervisors and workflow run recovery.

## Process cleanup

`ora-reaper` is a sidecar built from the same source revision. Every process created through the
production `TokioProcessSpawner` is registered with it over a private parent-liveness pipe. Normal
shutdown first releases process owners, then asks the reaper to force-kill survivors and waits for
acknowledgement; if Desktop crashes, the closed pipe triggers the same cleanup. Process trees are a
Unix process group or a Windows Job Object. See [`crates/process/README.md`](../../crates/process/README.md).

## Command layer

Command implementations live in `apps/desktop/src-tauri/src/commands/` by domain. Desktop owns the
build-time binding catalog in `bindings.rs` and `bindings/`: each unary binding names a logical
operation, its Rust handler and an explicit Webview permission; stream operations name a domain
startup handler and reuse the shared `stream_contract`/`cancel_contract_stream` pair. `task
export-contracts` joins this catalog with the logical operation catalog and generates the private
TypeScript map, typed stream routes, `app_commands.rs` and the permission files; ordinary Tauri
builds consume the checked-in output and never run `xtask`. See
[Frontend contracts](../core/frontend-contracts.md).

Access is granted per Webview label through `capabilities/`:

| Label | May invoke |
|---|---|
| `main` | Every registered command |
| `plugin-webview:*` | `plugin_webview_invoke` only, see [Surfaces](../plugins/surfaces.md) |
| `remote-webview:*` | Nothing: no capability targets the label, so the page has no Tauri IPC |

`build.rs` cross-checks the registry against `permissions/main-commands.toml`, so a registered
command cannot escape ACL enforcement.

### Request execution

A shared wrapper assigns a canonical request id, opens the request span, invokes the domain call,
projects any backend error to `{ code, params, requestId }` and records at most one completion
event. Commands call narrow Backend domain handles (`backend.workspaces()`, `backend.plugins()`,
`backend.sessions()`, `backend.workflow_runs()`, `backend.settings()`, and so on) rather than root
forwarding methods; the handles own their concurrency, locks and post-commit work, so Desktop never
sequences a domain's state transitions itself. Blocking domain work runs on the blocking executor.

The asynchronous executor (`run_async_backend`) takes every domain call as an already boxed future.
Tauri constructs each async command's future on the main thread inside the WebView IPC callback,
whose stack is small and already partly used; a deep domain future (the marketplace install chain
monomorphizes into hundreds of KB) would overflow it before the runtime polls it. Boxing keeps the
command future pointer-sized, and Desktop tests pin the plugin transfer commands against an explicit
future-size budget.

Streams are described in [Streams and cancellation](../core/streams.md).

### Native commands

Besides the generated contract commands, Desktop registers native commands that have no SDK
operation:

| Command | Purpose |
|---|---|
| `stream_contract`, `cancel_contract_stream` | Shared stream transport |
| `get_worktree_root`, `set_worktree_root` | Where new task worktrees are created |
| `resolve_task_cwd`, `resolve_workspace_cwd` | The live working directory of a task or Workspace |
| `open_location` | Reveal a path in the system file manager (`explorer /select,`, `open -R`) instead of opening it with the default editor; directories open as folder windows |
| `open_external_url` | Open an `http(s)`/`mailto` URL in the system browser; other schemes are `InvalidRequest` |
| `write_workflow_export`, `read_workflow_import` | Workflow file export and import |
| `download_today_log`, `download_plugin_log` | Copy a diagnostic log to a destination chosen in the save dialog |
| `surface_*`, `plugin_webview_invoke` | Plugin surfaces, see [Surfaces](../plugins/surfaces.md) |
| `get_desktop_update_status`, `check_desktop_update`, `install_desktop_update` | See [Desktop updates](#desktop-updates) |

## Persistent paths

| Data | Location |
|---|---|
| SQLite database (including `user_config`) | `<data root>/ora.sqlite3` |
| Logs | `<data root>/logs/ora.log` |
| Session history | `<data root>/sessions/` |
| Scheduled run artifacts | `<data root>/scheduled-artifacts/<run id>/` |
| Skill packages | `<data root>/atoms/skills/` |
| Instance lock | `<data root>/instance.lock` |
| Plugins, marketplace sources, plugin data and logs | `~/.ora/plugins/` |
| Default worktree root | `~/.ora/worktrees` |
| Downloaded update packages | `~/.ora/cache/desktop-updates/v2/` |

The worktree root is non-sensitive configuration chosen in Settings → Data & privacy; it must be an
absolute path to an existing directory. It is only a creation target: a new value affects task
creations that start afterwards, and existing worktrees are never moved or recomposed from it, see
[Tasks and worktrees](../workspaces/tasks-and-worktrees.md).

With `ORA_DATA_DIR` set, relative project and Workspace locations in the database resolve against
the data root's parent (the repository root under `task run:desktop`), not the process cwd, because
`tauri dev` starts in `apps/desktop/src-tauri`.

## Desktop updates

Releases ship for Windows (NSIS installer) and Linux (AppImage and `deb`). macOS is no longer
supported: v0.6.0 was the last macOS release, and the manifest has no macOS entry, so a macOS
installation stays on v0.6.0.

Release builds check the signed static manifest
`https://github.com/ora-space/desktop/releases/latest/download/latest.json` one minute after start
and then every six hours (`0 0 */6 * * *`) on an `ora-scheduler` job, using the user's configured proxy. Tauri's
updater selects the current OS and architecture and verifies the signature; the Desktop service
stores verified packages in an identity-addressed cache so a package downloaded by an earlier
process is reused (after re-verifying it against the configured public key) instead of downloaded
again. Development builds keep the commands for tests but register no network work.

- A package already downloaded stays installable when a later check fails; `Failed` is reported only
  when nothing was installable, and a failed installation restores `Ready`.
- Linux releases ship an AppImage, so a `deb`/`rpm` installation or a bare executable reports
  `ManualUpdate` with a reason before any download, and the shell points the user to the release
  instead of offering an install.
- The updater public key is in `apps/desktop/src-tauri/tauri.conf.json`; release builds need the
  matching private key in the GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`.

Status changes are emitted as `desktop-update-status-changed`. The lifecycle, cache layout and
invariants are in [`src/update/README.md`](../../apps/desktop/src-tauri/src/update/README.md).

## Bundled sidecars

Release builds bundle ripgrep and Deno next to the executable and resolve them at startup; Desktop
stops before creating application state if one is missing. Debug builds resolve `rg` and `deno` from
`PATH`. How the sidecars are downloaded and verified is in [Sidecars](../operations/sidecars.md).
