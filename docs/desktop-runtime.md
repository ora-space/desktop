# Desktop Runtime

Desktop command implementations live in `apps/desktop/src-tauri/src/commands/` by owning domain.
`commands.rs` contains only explicit module composition, command macros, and the shared synchronous
and asynchronous request execution mechanisms. Filesystem reads inject their Backend/file-reader
pair into the same executor; task and settings commands do not maintain private copies of that
lifecycle. Workspace listing, diff, and location operations share the workspace command module,
while Effect status belongs to its own module rather than plugin command implementation.

`apps/desktop/src-tauri` is the root Cargo workspace member that hosts Ora's persisted operations and ACP streaming capabilities through Tauri commands.

## Shared Backend and Commands

Desktop owns the build-time catalog in `apps/desktop/src-tauri/bindings.rs` and its
domain-scoped `bindings/` files. Each unary binding names one logical operation, its Rust
handler, and an explicit Webview permission. Streams name their domain startup handler;
the shared stream/cancel commands and platform-only commands have separate native bindings.
`task export-contracts` joins this catalog with the transport-neutral operation catalog and
generates the private TypeScript map, typed stream decoder/dispatcher, `app_commands.rs`, and
permission TOML files. The main and plugin Webview grants remain separate. Ordinary Tauri
builds consume these checked-in files and retain the existing handler/permission build check;
they never run xtask recursively. `task check:contracts` rejects drift and invalid bindings.

Desktop constructs one cloneable `ora-backend::Backend`. A shared command wrapper assigns a canonical
request id, opens the request span, invokes unary business logic, projects any backend error, and
records at most one completion event. Session load, prompt, and `watchAppEvents` operations use `stream_contract`, which
forwards ordered `data`, `error`, and `end` frames over a Tauri Channel. A private call id allows an
`AbortSignal` to cancel only that stream, while one separate request id correlates the complete stream.

The stream registry claims the call id before domain startup and retains it until its owning
registration is dropped. Cancellation signals that owner instead of freeing the id for reuse.
Startup already in progress is allowed to settle, because abandoning arbitrary domain work
could orphan actor side effects; a resource created after cancellation is immediately dropped.
The frontend signals cancellation even while startup is pending and repeats cleanup after it
settles. Application exit cancels all starting/running registrations and refuses new ones.
Domain modules supply event sources; they do not duplicate forwarding or request completion.

Agent-definition, Skill, and workflow-definition commands capture their corresponding Backend
domain handle for blocking execution. Their command declarations name the owning domain and
use case; neither the command executor nor root Backend gains a new forwarding method for each
operation. Prompted and automatic surface download imports both use the Skill interface.

Project and task commands use the same domain-handle path. Their aggregate deletes now enter the
shared async request executor as well: successful deletes and failures both complete the correlated
request once. Blocking cascades and post-commit cleanup notification are owned by their Backend
domain module, not sequenced by Desktop.

Workspace commands and file adapters capture `backend.workspaces()`. The handle owns workspace
queries, live path resolution, worktree-root persistence, and Git review; the generic filesystem
implementation remains in Desktop/`ora-fs`. A clone shares the existing root lock and Git cleanup
use leases, so readers and configuration changes still observe the same application state.

Plugin commands capture `backend.plugins()`, whose operations include the agent-set reconciliation
required after discovery or package mutations. The progress callback stays transport-owned, and
the surface host obtains its restricted gateway through `backend.plugins().gateway()`. Desktop
never coordinates a plugin mutation and a separate runtime sync itself.

Workflow-run commands capture `backend.workflow_runs()`. Start/restart/input mutations and
manual completion share the callback's run gate; terminal transitions commit before best-effort
session cleanup. Cancel and complete commands now use the same async request lifecycle wrapper
as other operations, including successful completion records. Startup recovery remains Backend-
composed but its implementation belongs to the workflow-run module.

Session commands and prompt/history stream startup capture `backend.sessions()`. Session title
edits commit before best-effort actor adoption and application invalidation. Workflow-owned prompt
admission and drop cleanup are injected into that interface as a restricted capability, so Desktop
never sequences workflow state transitions itself. App-event startup subscribes through
`backend.app_events()` without gaining a publisher.

Runtime readiness/model discovery and Effect status use their own handles as well. Git identity
uses the stateless Backend export through the same blocking request executor, without capturing
Desktop state. The command macros only support domain-owned operations; the transitional root
forwarding arms have been removed.

The frontend injects `createTauriTransport()` into `createContractsClient`. The transport maps contract operation names to Tauri commands and forwards the original request DTO unchanged. Backend failures use the direct `{ code, params, requestId }` payload without a public message or outer envelope. Local Tauri invocation failures never invent a request id.

Task workspace lookup is part of that shared contract surface. `get_task_workspace` returns the authoritative task root with an optional branch. `watchAppEvents` uses the same channel framing, cancellation, and exactly-once completion lifecycle as other Desktop streams.

Developer preferences use four unary commands in a separate settings command module: `get_developer_mode`, `set_developer_mode`, `get_runtime_log_level`, and `set_runtime_log_level`. They use the same lifecycle and error projection as other Desktop commands; no HTTP endpoint is involved.

Settings commands clone `backend.settings()` rather than the entire Backend. The same narrow
interface supplies the updater's configured proxy and the persisted startup logging preference;
the runtime log-level manager receives only its restricted preferred-level store. SQLite handles
and worktree-root persistence are not part of the public settings interface.

Backend construction immediately attempts one supervised connection per installed agent plugin; there is no other source of agents. Plugin processes are started and stopped by the plugin lifecycle, which the agent runtime attaches to rather than spawning its own. Sessions share the connection selected by their current `agentCli` while retaining their own ACP session id and Task worktree `cwd`. `switch_session_agent` moves a live conversation to another agent and `resume_session_history` recovers one whose history writes failed. Each agent retries independently; failures leave the Desktop shell and healthy agents available, while operations targeting an unavailable agent report `agent_runtime_unavailable`. Agent process discovery is owned entirely by each plugin — see [ACP Agent Runtime](agent-runtime.md).

Plugins of kind `ui` contribute surfaces: remote web sites shown in isolated native webviews. Desktop hosts them in `apps/desktop/src-tauri/src/surface/` on top of the `ora-surface` registry, exposes the `surface_*` commands to the main webview, emits `surface://event`, writes surface downloads into `<data-dir>/plugins/data/<namespace>/<name>/downloads/`, starts the plugin process on demand, and stops it 30 s after its last surface closes. Plugin processes have no filesystem access of their own; they read their data directory back through the `ora/storage/*` host methods served by `ora-plugin-lifecycle`. Disabling, stopping, or uninstalling a plugin closes its surfaces first through the lifecycle's `SurfaceCloser`. See [Plugin Surfaces](surface.md).

The Desktop App Shell waits for the `Ready` frame before mounting normal queries and watchers. The application stream is a multi-subscriber, best-effort session-title invalidation broadcast rather than a persisted event log.

Beyond the shared contract surface, Desktop registers four platform-only commands: `get_worktree_root`, `set_worktree_root`, `resolve_task_cwd`, and `open_location`. `open_location` with target `explorer` reveals a file in the system file manager (`explorer.exe /select,` on Windows, `open -R` on macOS) so the default file association — often Cursor — is not launched. Existing directories still open as folder windows.

## Desktop updates

Release builds register the Tauri updater and an in-process `ora-scheduler` job. The job performs
an initial delayed check and then checks the static GitHub Release manifest every six hours. The
manifest is `https://github.com/ora-space/desktop/releases/latest/download/latest.json`; Tauri
selects the current OS/architecture and verifies its signed updater artifact before the Desktop
service writes identity-addressed artifacts below `~/.ora/cache/desktop-updates/v2/`. Development
builds keep the commands available for integration tests but do not register network work. The
updater status is exposed by `get_desktop_update_status` and installation is started by
`install_desktop_update`; status changes are emitted as `desktop-update-status-changed`. A
successful current-version check removes the
committed artifact entry. On a later process start, a fresh manifest check can match that identity,
re-verify the package with the configured updater public key, and reuse it without another
download. The updater public key is configured in
`apps/desktop/src-tauri/tauri.conf.json`; release artifacts still require the matching private
key in the GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`.

A package that has already been downloaded stays installable: a later check that fails is logged
and leaves the `Ready` status in place, because the verified bytes are still on disk. `Failed` is
therefore only reported when nothing was installable to begin with, and a failed installation
restores `Ready` so the user can retry.

## Marketplace index refreshes

Desktop refreshes the cached marketplace registry index on its own: once fifteen seconds after
startup, and every six hours afterwards in the host's local time. Both run on the same
`ora-scheduler` and drive the same rebuild; only the trigger recorded in the log differs. Unlike
release updates, a refresh only rebuilds the cached listing and never installs or updates an
installed plugin, so development builds register it too rather than leaving it untested until a
packaged release. A refresh that fails is logged and not retried on its own: the next six-hour
tick is the retry.

The backend admits one index rebuild at a time. Because every rebuild produces the same index for
every caller, a caller that arrives while one is in flight is turned away rather than queued —
including a user pressing Sync, whose request is then answered from the cached index instead of
running a second identical rebuild. The shell disables its Sync action for the span of an
automatic refresh, reported as `marketplace-auto-sync-changed` with `started` and `finished`; the
admission is claimed before `started` is emitted, so the shell is never told a refresh began that
was in fact discarded. `finished` also invalidates the cached listing query, which is why the
shell mounts that bridge at its root rather than inside the settings dialog.

## Plugin marketplace artifact retrieval

Each configured plugin marketplace source independently selects how its `.orax` release artifacts
are retrieved. `Direct HTTPS` fetches an absolute HTTPS locator without request signing. `S3 SigV4`
accepts an object key, or a path-style HTTPS locator belonging to the configured endpoint and
bucket, and signs the request with that source's region and static credential pair. An S3 source
rejects foreign HTTPS locators instead of falling back to unsigned retrieval.

New sources start with Direct HTTPS. To use S3 SigV4, add the Git source first, then configure
artifact retrieval in its source editor. HTTPS release locators use the URL parser's normalization,
including case-insensitive schemes and surrounding ASCII whitespace.

The source editor exposes the modes as “HTTPS 直接获取” and “S3 签名获取”. S3 endpoint, bucket,
region, Access Key ID, and Secret Access Key are one complete configuration; existing credentials
are write-only and can be preserved or atomically replaced, but are never returned by the source
query contract or rendered in debug output. The current implementation stores this pair in the
local SQLite database as plaintext configuration. It does not yet provide OS-keychain encryption,
temporary credentials, or a provider credential chain.

Credential-update serialization carries plaintext credentials for IPC transport; never serialize
these requests into logs, traces, or telemetry. Use their redacted `Debug` representation for
diagnostics. Invalid S3 endpoints fail configuration preparation with a recoverable error.
Source updates return `marketplace_s3_credentials_required` when enabling S3 without credentials,
or `marketplace_artifact_retrieval_field_invalid` with a `field` parameter naming the invalid field
without its value. Corrupt persisted retrieval configuration is reported as an internal error.

Both modes keep the source's proxy selection and the common download pipeline. SigV4 authenticates
the request but does not establish package integrity: every artifact must still pass the release
manifest's SHA-256 before installation.

The static manifest advertises an AppImage for Linux, which the updater can only install into an
AppImage installation. A `deb` or `rpm` installation, or a build running as a bare executable, is
reported as `ManualUpdate` with the reason instead, before any download is spent; the shell then
names the release and the channel to update through rather than offering an install action.

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

The Tauri identifier is `space.ora.desktop`. Desktop keeps persistent state in two explicitly named roots:

- SQLite: `app_data_dir/ora.sqlite3`
- User configuration, including `worktree_root` and `network_proxy_settings`: `app_data_dir/ora.sqlite3`, table `user_config`
- Logs: `app_data_dir/logs/ora.log`
- Default new-worktree root: `~/.ora/worktrees`
- Session history: `app_data_dir/sessions`
- Skill packages root: `app_data_dir/atoms/skills`
- Plugins, registry data, and plugin storage: `~/.ora/plugins`

On first launch, Desktop creates the app data directory and `~/.ora/worktrees`. A worktree root already selected in SQLite takes precedence over that default.

`ORA_DATA_DIR` controls Desktop's runtime data root. `task run:desktop` points it at the repo `.data` directory for local development. Relative project roots stored in that database are resolved against the data directory's parent (the repo root), not the Tauri process cwd — `tauri dev` starts in `apps/desktop/src-tauri`, which would otherwise miss paths such as `.data/rustun`. Without `ORA_DATA_DIR`, runtime data paths come from Tauri's `app_data_dir`; the first-run worktree root remains `~/.ora/worktrees`, and folder-picker selections are already absolute.

The worktree root is non-sensitive configuration. Users can change it from Settings → Data & privacy on Desktop. A selected value must be an absolute path to an existing directory. The new value affects task creations that start after the update; in-flight operations retain their original snapshot, and existing worktrees are not moved.

The configured root is only a creation target. Existing worktree locations are resolved from the stored branch name and `git worktree list --porcelain` when an agent Session starts or loads, and `resolve_task_cwd` exposes that same resolution to the shell. Task and project deletion never mutate Git. See [Task Worktrees](task-worktrees.md).

## Logging

Desktop initializes `ora-logging` before opening the backend and registers the Gitlancer logger bridge. It accepts `ORA_LOG_LEVEL` as a process-only startup override; otherwise it restores the SQLite `log_level` preference, defaulting to `info`. The reload control and persistence adapter are composed through `ora-runtime-settings`, which serializes updates and compensates the live filter when persistence fails. Logs rotate daily and retain three files. Debug builds write to stdout and the file; release builds write to the file only. The logging guard remains managed for the application lifetime.

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

The Tauri Rust crate shares the root `Cargo.lock`, dependency graph, and target directory with the reusable Rust crates. `task test:frontend` includes the Desktop TypeScript transport tests, while `task test:crates`, `task test:tauri`, and `task test:e2e` separately cover reusable crates, the Tauri package, and Desktop E2E tests. `task test` runs all four groups; CI runs the groups independently.
