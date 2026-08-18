# Desktop Runtime

`apps/desktop/src-tauri` is the root Cargo workspace member that hosts Ora's persisted operations and ACP streaming capabilities through Tauri commands.

## Shared Backend and Commands

Desktop constructs one cloneable `ora-backend::Backend`. A shared command wrapper assigns a canonical
request id, opens the request span, invokes unary business logic, projects any backend error, and
records at most one completion event. Session load, prompt, and `watchAppEvents` operations use `stream_contract`, which
forwards ordered `data`, `error`, and `end` frames over a Tauri Channel. A private call id allows an
`AbortSignal` to cancel only that stream, while one separate request id correlates the complete stream.

The frontend injects `createTauriTransport()` into `createContractsClient`. The transport maps contract operation names to Tauri commands and forwards the original request DTO unchanged. Backend failures use the direct `{ code, params, requestId }` payload without a public message or outer envelope. Local Tauri invocation failures never invent a request id.

Task workspace lookup and Spec review are part of that shared contract surface. `get_task_workspace` returns the authoritative task root with an optional branch, while `get_spec_catalog` and `read_spec` delegate unary work to the shared backend. `watch_specs` and `watchAppEvents` use the same channel framing, cancellation, and exactly-once completion lifecycle as other Desktop streams.

Backend construction immediately attempts supervised `opencode acp`, `nga acp`, `codeagentcli acp`, `claude-agent-acp`, and `codex-acp` children in the user's home directory. Sessions share the connection selected by their current `agentCli` while retaining their own ACP session id and Task worktree `cwd`. `switch_session_agent` moves a live conversation to another CLI and `resume_session_history` recovers one whose history writes failed. Each CLI retries independently; failures leave the Desktop shell and healthy CLIs available, while operations targeting an unavailable CLI report `agent_runtime_unavailable`. Executable lookup is platform-specific — see [ACP Agent Runtime](agent-runtime.md).

The Desktop App Shell waits for the `Ready` frame before mounting normal queries and watchers. The application stream is a multi-subscriber, best-effort session-title invalidation broadcast rather than a persisted event log.

Beyond the shared contract surface, Desktop registers four platform-only commands: `get_desktop_config`, `set_worktree_root`, `resolve_task_cwd`, and `open_location`.

## Skill imports

Desktop exposes the shared import session lifecycle through four unary Tauri commands:
`prepare_skill_import`, `get_skill_import`, `commit_skill_import`, and `cancel_skill_import`.
The frontend sends the system picker's local path inside `PrepareSkillImportRequest`; the Rust
side reads the folder or archive, so file bytes (up to 200 MiB) are never serialized over Tauri
IPC. Preparation, preview, conflict decisions, background commit, and result retention behave
through the shared `ora-backend` composition. Before the catalog row is committed, package files,
journal markers, and directory promotes are flushed on platforms that support directory fsync.
Mutation-time directory fsync is a hard error on Unix and best-effort on Windows; macOS
`sync_all` uses `fsync` rather than `F_FULLFSYNC`.

The configured root is only a creation target. Existing worktree locations are resolved from the stored branch name and `git worktree list --porcelain` when an agent Session starts or loads. Task and project deletion never mutate Git.

## Persistent Paths

The Tauri identifier is `space.ora.desktop`. Tauri's system `app_data_dir` owns all default runtime state:

- SQLite: `app_data_dir/ora.sqlite3`
- Configuration: `app_data_dir/config.json`
- Logs: `app_data_dir/logs/ora.log`
- Default new-worktree root: `~/.ora/worktrees`
- Session history: `app_data_dir/sessions`
- Skill packages root: `app_data_dir/atoms/skills`

On first launch, Desktop creates the app data directory, `~/.ora/worktrees`, and a versioned configuration file using an atomic sibling-temporary-file replacement. `config.json` currently holds version `1` and the `worktreeRoot`. Existing malformed, unknown-version, or otherwise invalid configuration is fatal; Desktop does not silently reset it.

`ORA_DATA_DIR` controls Desktop's runtime data root. `task run:desktop` points it at the repo `.data` directory for local development. Relative project roots stored in that database are resolved against the data directory's parent (the repo root), not the Tauri process cwd — `tauri dev` starts in `apps/desktop/src-tauri`, which would otherwise miss paths such as `.data/rustun`. Without `ORA_DATA_DIR`, runtime data paths come from Tauri's `app_data_dir`; the first-run worktree root remains `~/.ora/worktrees`, and folder-picker selections are already absolute.

The worktree root is non-sensitive configuration. Users can change it from Settings → Data & privacy on Desktop. A selected value must be an absolute path to an existing directory. The new value affects task creations that start after the update; in-flight operations retain their original snapshot, and existing worktrees are not moved.

The configured root is only a creation target. Existing worktree locations are resolved from the stored branch name and `git worktree list --porcelain` when an agent Session starts or loads, and `resolve_task_cwd` exposes that same resolution to the shell. Task and project deletion never mutate Git. See [Task Worktrees](task-worktrees.md).

## Logging

Desktop initializes `ora-logging` before opening the backend and registers the Gitlancer logger bridge. Logs rotate daily and retain three files. Debug builds write to stdout and the file; release builds write to the file only. The logging guard remains managed for the application lifetime.

Each unary command or stream emits at most one request-completion event using the same request id as
its public failure payload or error frame. Cancellation is completed at `DEBUG` and is not projected
as `internal_error`. Git worktree and branch cleanup after a failed task creation or a deletion is
never attempted inline: it is queued as a durable `git_cleanup` job and executed later by the shared
backend cleanup worker (see [Task Worktrees](task-worktrees.md)), so the primary response and source
chain are unaffected by how that later cleanup turns out. The worker logs its own outcomes under
`operation = "git_cleanup"`, independent of the originating request id.

At startup, Desktop reads the operating system's IANA timezone and fixes it for the process
lifetime. Structured event timestamps use that timezone. If the system timezone cannot be read or
parsed, Desktop records a warning, uses UTC, and continues startup. A system timezone change takes
effect after Ora restarts. Daily log files continue to rotate at UTC boundaries.

## Verification

The Tauri Rust crate shares the root `Cargo.lock`, dependency graph, and target directory with the reusable Rust crates. `task test:frontend` includes the Desktop TypeScript transport tests, while `task test:crates` includes `ora-desktop` alongside every other Rust workspace package. `task test` runs both groups.
