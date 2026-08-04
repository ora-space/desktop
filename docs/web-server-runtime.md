# Web Server Runtime

`apps/web/server` is Ora's HTTP backend runtime.

## Purpose

- It boots shared structured logging through `ora-logging` and registers the Gitlancer command-logging bridge.
- It exposes health endpoints for process liveness and runtime readiness.
- It serves persisted HTTP operations for Project, Task, Session, Skill, Agent, and Git identity through the shared `ora-backend` composition.
- It provisions task-owned linked worktrees during creation and leaves Git untouched during deletion.
- It streams ACP load replay and prompt updates as bounded NDJSON responses.
- It provides read-only server filesystem listings for the Web platform path picker.
- It provides a task-scoped, read-only workspace explorer with bounded file reads, ripgrep search, and native refresh events.
- It owns the project work context routes, which are outside `ora-backend`.

## Data root configuration

The web server reads one runtime data root:

- `ORA_DATA_DIR`: root directory for runtime state. Default: `.`, resolved to an absolute path against the process working directory so Git commands running elsewhere still resolve it correctly. A blank value fails startup.

Every other runtime path is derived from it — there is no separate variable for any of them:

- SQLite database: `<ORA_DATA_DIR>/ora.sqlite3`
- Worktree creation root: `<ORA_DATA_DIR>/worktrees`
- Imported skill folders: `<ORA_DATA_DIR>/atoms/skills`
- Log file: `<ORA_DATA_DIR>/logs/ora.log`
- Session history root: `<ORA_DATA_DIR>/sessions`

Two Ora processes must not share one data root. SQLite tolerates it, but session history files are written on a single-writer assumption that only holds within one process. See [ACP Agent Runtime](agent-runtime.md).

Startup asks `ora-backend` to create the required directories, bootstrap the database, apply the active migration catalog, and construct the shared composition before the runtime is marked ready. A SQLite database that cannot be opened, migrated, or pooled fails startup with a typed bootstrap error rather than serving requests from a partially initialized runtime. The server retains direct composition only for the Web-only project work context and filesystem services.

## Project configuration

The web server also requires a bootstrap project identity:

- `ORA_PROJECT_NAME`: persisted workspace project name. Required.
- `ORA_PROJECT_PATH`: persisted workspace root path. Required.

A blank or missing value for either fails startup with a typed bootstrap error rather than serving requests with an unknown workspace identity.

Startup reconciles this configured project into the `projects` table before the runtime is marked ready:

- If no visible project exists with the configured name, startup creates one row.
- If a visible project exists with the configured name but a different stored path, startup fails, because project roots are immutable.
- If both the configured name and path already match, startup leaves the row unchanged.

After reconciliation, startup opens the synthetic web work context `surface = web`, `window_id = main` for that project and refreshes its lease immediately.

Task creation resolves the project named in the request and provisions linked worktrees under `<ORA_DATA_DIR>/worktrees/<full-task-id>`. Agent session startup instead resolves Task → Worktree → branch name and then asks Git for the authoritative linked-worktree path, which becomes the ACP session `cwd`. See [Task Worktrees](task-worktrees.md).

## Bind configuration

- `ORA_HOST`: bind host. Default: `0.0.0.0`
- `ORA_PORT`: bind port. Default: `32578`

When unset, the server binds `0.0.0.0:32578`. An invalid host or port fails startup with a typed bootstrap error rather than falling back to an unexpected listener address.

Logging variables are documented in [Runtime Logging](runtime-logging.md).

## Health endpoints

- `GET /health/live`: confirms that the process is running
- `GET /health/ready`: confirms that application-state bootstrap completed successfully

`/health/ready` does not return success until the runtime finishes constructing its application state.

## HTTP API

Route paths come from the `ora-contracts` endpoint manifest constants, so a route and its generated client entry cannot drift apart. Path parameters are camelCase.

### project

- `POST /api/projects`
- `GET /api/projects`
- `GET /api/projects/{projectId}`
- `PUT /api/projects/{projectId}`
- `DELETE /api/projects/{projectId}`

### projectWorkContext

- `POST /api/project-work-contexts/open`
- `POST /api/project-work-contexts/renew`

### task

- `POST /api/tasks`
- `GET /api/tasks`
- `GET /api/tasks/{taskId}`
- `PUT /api/tasks/{taskId}`
- `DELETE /api/tasks/{taskId}`

### session

- `POST /api/sessions`
- `GET /api/sessions`
- `GET /api/sessions/{sessionId}`
- `POST /api/sessions/{sessionId}/load`
- `POST /api/sessions/{sessionId}/prompt`
- `POST /api/sessions/{sessionId}/permissions/respond`
- `POST /api/sessions/{sessionId}/stop`
- `POST /api/sessions/{sessionId}/agent`
- `POST /api/sessions/{sessionId}/history/resume`
- `DELETE /api/sessions/{sessionId}`

### agentRuntime

- `GET /api/agent-models`

### skill

- `POST /api/skills`
- `GET /api/skills`
- `GET /api/skills/{skillId}`
- `PUT /api/skills/{skillId}`
- `DELETE /api/skills/{skillId}`
- `POST /api/skills/import`

### agent

- `POST /api/agents`
- `GET /api/agents`
- `GET /api/agents/{agentId}`
- `PUT /api/agents/{agentId}`
- `DELETE /api/agents/{agentId}`

### fileSystem

- `GET /api/file-system/directory?path={absolute_path}`
- `GET /api/projects/{projectId}/branches`

### task workspace files

- `POST /api/tasks/{taskId}/files/list` with `{ "path": "src" }` to list one workspace-relative directory
- `POST /api/tasks/{taskId}/files/read` with `{ "path": "src/main.rs" }` to read one bounded UTF-8 file
- `POST /api/tasks/{taskId}/files/search` with `{ "query": "needle", "kind": "files" | "content" }` to search filenames or text
- `GET /api/tasks/{taskId}/files/watch` to stream debounced `WorkspaceFileEventBatch` NDJSON frames

Every task workspace route resolves the active task workspace through `ora-backend`; the client cannot select a root directory. The filesystem service rejects absolute paths, parent traversal, and symlink escapes. File reads are capped at 10 MiB and reject binary or invalid UTF-8 content. Search uses fixed-string matching for content, a 15-second timeout, an 8 MiB output bound, and at most 10,000 results. Watch events are coalesced for 100 ms before a batch is emitted. See [Task Workspace Files](task-workspace-files.md).

### gitIdentity

- `GET /api/git/identity`

Each route translates transport input into the matching `ora-contracts` request DTO, delegates to the shared backend, and serializes the returned contract response without adding adapter-local response shapes.

Task payloads do not expose backend-owned worktree identifiers, and the runtime exposes no standalone public worktree endpoints — `/api/worktrees` and `/api/worktrees/{worktreeId}` are not part of the API.

Project branch responses separate the logical branch name, exact resolvable ref, and display label. Ora-managed branches use their task titles as labels while preserving the Git ref used to create a new worktree.

`GET /api/git/identity` returns the host's Git identity for the sidebar profile: the global Git config first, falling back to the authenticated GitHub CLI account when Git has no name configured.

### Agent runtime

Backend construction immediately attempts one supervised `acp` child per supported CLI, rooted at the user's home directory. Executable resolution is platform-specific: on Unix each CLI is read from its fixed per-user directory (`<home>/.opencode/bin/opencode`, `<home>/.nga/bin/nga`, `<home>/.codeagentcli/bin/codeagentcli`); on Windows it is resolved from `PATH` through `where.exe` on every retry generation.

Each independent supervisor performs `initialize` once per process generation and retries failures without blocking healthy CLIs or non-agent APIs. Session create calls `session/new` on the connection selected by `agentCli` and returns the latest available-command catalog announced during setup. Updates emitted before the response reveals the private provider session id are temporarily buffered, then attached to the matching session route. Load calls `session/load` using that private id and the Task worktree `cwd`; the public Session payload never exposes it. The load response streams Ora's own recorded conversation rather than the agent's replay, which is drained and discarded. `POST /api/sessions/{sessionId}/agent` rebinds a live conversation to a different CLI without changing its identifier, and `POST /api/sessions/{sessionId}/history/resume` returns a session whose history writes failed to a writable state. `GET /api/agent-models` concurrently runs each CLI's bounded `models` discovery command and returns only successful groups.

Prompt requests carry an ordered ACP content-block list and may combine text, images, audio, and resources. The serialized prompt is limited to 16 MiB. Load and prompt responses use `application/x-ndjson`; each line is one complete frame. Data and control paths are separate, session-update queues are bounded at 256 items, frames are limited to 8 MiB, and overflow terminates the operation rather than dropping updates silently. See [ACP Agent Runtime](agent-runtime.md).

Unary requests and streams receive a server-generated canonical request id before entering business logic. Client-provided `X-Request-Id` values are ignored. Every Web response publishes the canonical id through `X-Request-Id`, CORS exposes that header, and a failure body or error frame carries the same id in its direct `{ code, params, requestId }` payload. A stream keeps one id from creation through normal completion, failure, disconnect, or cancellation.

### Skill folder import

`POST /api/skills/import` uploads a local skill folder as one `multipart/form-data` request. Each file part carries its skill-root-relative path as the part file name. The request body is capped at 50 MB: oversized uploads return the typed `413` `skill_upload_too_large` error, while uploads over 1000 files return `422`.

The import is atomic across the filesystem and the database:

- Files stream into `<ORA_DATA_DIR>/atoms/skills/<uuid>.tmp`, where `<uuid>` is the skill's future identifier.
- Root `SKILL.md` front matter supplies the persisted `name` and `description`; the committed directory uses the validated name.
- The SQLite row insert, staging-directory promotion, and transaction commit execute as one unit of work. A primary failure keeps its full Rust source chain for the correlated completion log, while any compensating cleanup failure is logged separately without replacing that primary error.
- Empty uploads, excessive file counts, unsafe or duplicate relative paths, and invalid manifests return typed `422` contract errors. An existing committed directory returns the typed `409` `skill_folder_conflict` error.
- Deleting a skill also removes its committed directory. Startup reconciliation removes leftover staging directories and committed directories with no visible database row before the backend becomes ready.

### Project work contexts

- `open` creates or switches one `(surface, window_id)` context into a project and refreshes its lease immediately.
- `renew` extends an existing context lease using backend time.
- Occupied-project conflicts return a stable HTTP `409` without exposing the owning surface or window id in the response.

See [Project Work Contexts](project-work-contexts.md) for lease timing and current wiring.

### Filesystem browsing

The filesystem directory route supports the custom Web path picker.

- Omitting `path` lists the Web Server process user's home directory.
- Supplied paths must be absolute. Relative paths return `invalid_file_system_path`.
- Responses include the current path, parent path, server-derived breadcrumbs, and all child entries.
- Hidden entries are included. Symbolic links remain visible and preserve their link paths; broken links are reported as unavailable entries.
- Directories sort before files, and the endpoint returns the complete directory without pagination.
- The route intentionally has no configured browse root and can navigate outside home. Deployments must account for the exposed server directory metadata when setting network access to the Web Server.

## Error semantics

Error mapping is centralized so application outcomes become stable HTTP responses instead of leaking internal formatting.

- A not-found outcome from any project, task, session, skill, or agent get, update, or delete route returns an HTTP not-found status with a structured error payload identifying the missing entity family.
- A repository or bootstrap failure returns an HTTP server-error status with a structured error payload rather than raw infrastructure error text.
- Task-create failures caused by linked-worktree provisioning or compensating cleanup return a structured server error identifying task creation as failed, without exposing Git command output or filesystem-specific formatting.
- An occupied project returns `409`.
- Skill upload body limits return `413`; file-set and manifest validation return `422`; an existing committed skill directory returns `409`. All use the same typed contract body and canonical request id as other failures.

Shared backend failures project to the same typed `{ code, params, requestId }` payload used by Desktop; there is no public message or outer error envelope. HTTP status derives from the backend error classification.

Task workspace failures use the same mapping: missing task or workspace path returns a typed not-found response, invalid relative paths and unreadable text return a typed bad request, and bounded-output failures retain their native `413`/`422` classification. Watch failures are emitted as `{ "type": "error", "error": { "code", "params", "requestId" } }` frames rather than raw filesystem messages.

## Frontend development modes

- `task run:web-backend` starts the Rust HTTP backend on its default port.
- `task run:web-frontend` starts Vite with the fetch contracts transport and expects the backend to run separately.

The Web frontend always uses the fetch contracts transport and talks to the Rust HTTP backend, in both development and production builds.

Long-lived task workspace watch responses defer completion until an end or error frame so the request id, error payload, and log event remain correlated. This keeps the watcher aligned with the ACP stream lifecycle and prevents an early success event followed by a duplicate failure event.

## Storage behavior

The runtime uses a file-backed SQLite database bootstrapped through `ora-db`.

- Data persists across process restarts as long as the same `ORA_DATA_DIR` is reused.
- Readiness depends on successful database bootstrap, repository-pool construction, bootstrap-project reconciliation, and synthetic web work context reconciliation.
- The request seam emits at most one correlated completion event. Ordinary success is `INFO`, health and readiness success are `DEBUG`, and failure levels derive from the shared backend classification.
