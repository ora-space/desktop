# Desktop Runtime

`apps/desktop/src-tauri` is an independent Cargo workspace that hosts the same persisted operations and ACP streaming capabilities as the Web server without running an HTTP server.

## Shared Backend and Commands

Desktop constructs one cloneable `ora-backend::Backend`. A shared command wrapper assigns a canonical
request id, opens the request span, invokes unary business logic, projects any backend error, and
records at most one completion event. Session load and prompt operations use `stream_contract`, which
forwards ordered `data`, `error`, and `end` frames over a Tauri Channel. A private call id allows an
`AbortSignal` to cancel only that stream, while one separate request id correlates the complete stream.

The frontend injects `createTauriTransport()` into `createContractsClient`. The transport maps contract operation names to Tauri commands and forwards the original request DTO unchanged. Shared backend failures use the same direct `{ code, params, requestId }` payload as Web, without a public message or outer envelope. Tauri and fetch reuse the same runtime decoder; local Tauri invocation failures have no HTTP status and never invent a request id.

Backend construction immediately attempts supervised `opencode acp`, `nga acp`, and `codeagentcli acp` children in the user's home directory. Sessions share the connection selected by their current `agentCli` while retaining their own ACP session id and Task worktree `cwd`. `switch_session_agent` moves a live conversation to another CLI and `resume_session_history` recovers one whose history writes failed. Each CLI retries independently; failures leave the Desktop shell and healthy CLIs available, while operations targeting an unavailable CLI report `agent_runtime_unavailable`. Executable lookup is platform-specific — see [ACP Agent Runtime](agent-runtime.md).

Beyond the shared contract surface, Desktop registers four platform-only commands with no HTTP counterpart: `get_desktop_config`, `set_worktree_root`, `resolve_task_cwd`, and `open_location`.

Three contract operations are not implemented on Desktop:

- opening a project work context;
- renewing a project work context;
- listing a server filesystem directory.

No Tauri command exists for them. The contracts transport rejects them with `unsupported_operation` before any IPC call is made, so the exclusion is enforced client-side rather than by a stub command. `ProjectWorkContext` remains outside this extraction; see [Project Work Contexts](project-work-contexts.md).

## Persistent Paths

The Tauri identifier is `space.ora.desktop`. Tauri's system `app_data_dir` owns all default runtime state:

- SQLite: `app_data_dir/ora.sqlite3`
- Configuration: `app_data_dir/config.json`
- Logs: `app_data_dir/logs/ora.log`
- Default new-worktree root: `app_data_dir/worktrees`
- Session history: `app_data_dir/sessions`

On first launch, Desktop creates the app data directory, default worktree directory, and a versioned configuration file using an atomic sibling-temporary-file replacement. `config.json` currently holds version `1` and the `worktreeRoot`. Existing malformed, unknown-version, or otherwise invalid configuration is fatal; Desktop does not silently reset it.

Unlike the Web server, Desktop reads no environment variables for these paths. Everything is derived from Tauri's `app_data_dir` and the versioned configuration file.

The worktree root is non-sensitive configuration. Users can change it from Settings → Data & privacy on Desktop. A selected value must be an absolute path to an existing directory. The new value affects task creations that start after the update; in-flight operations retain their original snapshot, and existing worktrees are not moved.

The configured root is only a creation target. Existing worktree locations are resolved from the stored branch name and `git worktree list --porcelain` when an agent Session starts or loads, and `resolve_task_cwd` exposes that same resolution to the shell. Task and project deletion never mutate Git. See [Task Worktrees](task-worktrees.md).

## Logging

Desktop initializes `ora-logging` before opening the backend and registers the Gitlancer logger bridge. Logs rotate daily and retain three files. Debug builds write to stdout and the file; release builds write to the file only. The logging guard remains managed for the application lifetime.

Each unary command or stream emits at most one request-completion event using the same request id as
its public failure payload or error frame. Cancellation is completed at `DEBUG` and is not projected
as `internal_error`. If rollback or cleanup also fails, Desktop retains the primary response and
source chain and records the secondary failure as a separate operation with the same request id.

At startup, Desktop reads the operating system's IANA timezone and fixes it for the process
lifetime. Structured event timestamps use that timezone. If the system timezone cannot be read or
parsed, Desktop records a warning, uses UTC, and continues startup. A system timezone change takes
effect after Ora restarts. Daily log files continue to rotate at UTC boundaries.

## Verification

The Tauri Rust crate keeps its own `Cargo.lock` and is intentionally excluded from the root Cargo workspace. `task test:desktop` checks the Desktop transport, formatting, Clippy, and the independent Rust tests. `task test` includes this task explicitly.
