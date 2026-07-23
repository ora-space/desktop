# Desktop Runtime

`apps/desktop/src-tauri` is an independent Cargo workspace that hosts the same persisted operations and ACP streaming capabilities as the Web server without running an HTTP server.

## Shared Backend and Commands

Desktop constructs one cloneable `ora-backend::Backend`. Unary operations use typed snake-case Tauri commands. Session load and prompt operations use `stream_contract`, which forwards ordered `data`, `error`, and `end` frames over a Tauri Channel. A private call id allows an `AbortSignal` to cancel only that stream.

The frontend injects `createTauriTransport()` into `createContractsClient`. The transport maps contract operation names to Tauri commands and forwards the original request DTO unchanged. Shared backend errors retain the same public code and message as Web errors; Tauri transport errors have no HTTP status.

The current Desktop slice explicitly returns `unsupported_operation` for:

- opening a project work context;
- renewing a project work context;
- listing a server filesystem directory.

`ProjectWorkContext` remains outside this extraction.

## Persistent Paths

The Tauri identifier is `space.ora.desktop`. Tauri's system `app_data_dir` owns all default runtime state:

- SQLite: `app_data_dir/ora.sqlite3`
- Configuration: `app_data_dir/config.json`
- Logs: `app_data_dir/logs/ora.log`
- Default new-worktree root: `app_data_dir/worktrees`

On first launch, Desktop creates the app data directory, default worktree directory, and a versioned configuration file using an atomic sibling-temporary-file replacement. Existing malformed, unknown-version, or otherwise invalid configuration is fatal; Desktop does not silently reset it.

The worktree root is non-sensitive configuration. Users can change it from Settings → Data & privacy on Desktop. A selected value must be an absolute path to an existing directory. The new value affects task creations that start after the update; in-flight operations retain their original snapshot, and existing worktrees are not moved.

The configured root is only a creation target. Existing worktree locations are resolved from the stored branch name and `git worktree list --porcelain` when an agent Session starts or loads. Task and project deletion never mutate Git.

## Logging

Desktop initializes `ora-logging` before opening the backend and registers the Gitlancer logger bridge. Logs rotate daily and retain three files. Debug builds write to stdout and the file; release builds write to the file only. The logging guard remains managed for the application lifetime.

At startup, Desktop reads the operating system's IANA timezone and fixes it for the process
lifetime. Structured event timestamps use that timezone. If the system timezone cannot be read or
parsed, Desktop records a warning, uses UTC, and continues startup. A system timezone change takes
effect after Ora restarts. Daily log files continue to rotate at UTC boundaries.

## Verification

The Tauri Rust crate keeps its own `Cargo.lock` and is intentionally excluded from the root Cargo workspace. `task test:desktop` checks the Desktop transport, formatting, Clippy, and the independent Rust tests. `task test` includes this task explicitly.
