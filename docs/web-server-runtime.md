# Web Server Runtime

`apps/web/server` is the first HTTP backend runtime for Ora.

## Purpose

- It boots shared structured logging through `ora-logging`.
- It exposes health endpoints for process liveness and runtime readiness.
- It serves persisted HTTP operations for Project, Task, Session, Skill, and Agent through the shared `ora-backend` composition.
- It provisions task-owned linked worktrees during creation and leaves Git untouched during deletion.
- It streams ACP load replay and prompt updates as bounded NDJSON responses.
- It provides read-only server filesystem listings for the Web platform path picker.

## Database Configuration

The web server reads its runtime data root from:

- `ORA_DATA_DIR`: root directory for runtime state. Default: `.`

Startup asks `ora-backend` to bootstrap the database, apply the active migration catalog, and construct the shared CRUD composition before the runtime is marked ready. The server retains direct composition only for the Web-only project work context and filesystem services.

- SQLite database path: `<ORA_DATA_DIR>/ora.sqlite3`
- Worktree root: `<ORA_DATA_DIR>/worktrees`
- Imported skill folders: `<ORA_DATA_DIR>/atoms/skills`
- Log file: `<ORA_DATA_DIR>/logs/ora.log`

## Project Configuration

The web server also requires a bootstrap project identity:

- `ORA_PROJECT_NAME`: persisted workspace project name. Required.
- `ORA_PROJECT_PATH`: persisted workspace root path. Required.

Startup reconciles this configured project into the `projects` table before the runtime is marked ready.

- If no visible project exists with the configured name, startup creates one row.
- If a visible project exists with the configured name but a different stored path, startup fails because project roots are immutable.
- If both the configured name and path already match, startup leaves the row unchanged.
- If `ORA_WORK_DIR` is unset, startup uses a `worktrees/` directory next to the configured SQLite database file.
- Task creation resolves the project named by the request and provisions linked worktrees under `ORA_WORK_DIR/<full-task-id>`.
- Agent Session startup resolves Task → Worktree → branch and then asks Git for the authoritative linked-worktree path to use as the child process cwd.
- After project reconciliation, startup also opens the synthetic web work context `surface = web`, `window_id = main` for that project and refreshes its lease immediately.

## Bind Configuration

The web server reads its listener configuration from:

- `ORA_HOST`: bind host. Default: `0.0.0.0`
- `ORA_PORT`: bind port. Default: `32578`

When unset, the server binds `0.0.0.0:32578`.

Invalid host or port values fail startup during bootstrap.

## Health Endpoints

- `GET /health/live`: confirms that the process is running
- `GET /health/ready`: confirms that application-state bootstrap completed successfully

`/health/ready` remains unavailable until the runtime finishes constructing its application state.

## HTTP API

The persisted runtime exposes CRUD routes for the supported public models:

- `POST /api/projects`
- `GET /api/projects`
- `GET /api/projects/{project_id}`
- `PUT /api/projects/{project_id}`
- `DELETE /api/projects/{project_id}`
- `POST /api/project-work-contexts/open`
- `POST /api/project-work-contexts/renew`
- `POST /api/tasks`
- `GET /api/tasks`
- `GET /api/tasks/{task_id}`
- `PUT /api/tasks/{task_id}`
- `DELETE /api/tasks/{task_id}`
- `POST /api/sessions`
- `GET /api/sessions`
- `GET /api/sessions/{session_id}`
- `POST /api/sessions/{session_id}/load`
- `POST /api/sessions/{session_id}/prompt`
- `POST /api/sessions/{session_id}/permissions/respond`
- `POST /api/sessions/{session_id}/stop`
- `DELETE /api/sessions/{session_id}`
- `POST /api/skills`
- `GET /api/skills`
- `GET /api/skills/{skill_id}`
- `PUT /api/skills/{skill_id}`
- `DELETE /api/skills/{skill_id}`
- `POST /api/skills/import`
- `POST /api/agents`
- `GET /api/agents`
- `GET /api/agents/{agent_id}`
- `PUT /api/agents/{agent_id}`
- `DELETE /api/agents/{agent_id}`
- `GET /api/file-system/directory?path={absolute_path}`

Request and response payloads use `ora-contracts` DTO shapes, so transport behavior stays aligned with the shared application contract.
Task payloads do not expose backend-owned worktree identifiers, and the runtime does not expose standalone public worktree CRUD endpoints.

Session create starts `<home>/.opencode/bin/opencode acp`, `<home>/.nga/bin/nga acp`, or `<home>/.codeagentcli/bin/codeagentcli acp` according to the immutable `agentCli`. The server performs `initialize` and `session/new` before persisting. Load performs a fresh `initialize` followed by `session/load` using the private provider session id. The public Session payload never exposes that id.

Load and prompt responses use `application/x-ndjson`. Each line is one complete frame. Data and control paths are separate, session-update queues are bounded at 256 items, frames are limited to 8 MiB, and overflow terminates the operation rather than dropping updates silently.

### Skill Folder Import

`POST /api/skills/import` uploads a local skill folder as one `multipart/form-data` request. Each
file part carries its skill-root-relative path as the part file name, and the request body is capped
at 50 MB (oversized uploads return `413`, uploads over 1000 files return `422`).

The import is atomic across the filesystem and the database:

- Files stream into a staging directory `<ORA_DATA_DIR>/atoms/skills/<uuid>.tmp`, where `<uuid>` is
  the skill's future identifier.
- The staged `SKILL.md` front matter supplies the skill `name` and `description`; the committed
  directory is named after that resolved `name`.
- The row insert, the staging→`<name>` directory rename, and the transaction commit run as one unit
  of work, so a failure at any point leaves neither a committed row nor a promoted directory.
- Validation failures return `422`, and importing a folder whose resolved name already has a
  committed directory returns `409`.
- Deleting a skill also removes its committed directory. Startup reconciliation discards leftover
  `*.tmp` staging directories and any committed directory not backed by a visible skill row,
  restoring a clean layout after a crash.

The project work context routes provide the current backend-managed project selection surface.

- `open` creates or switches one `(surface, window_id)` context into a project and refreshes its lease immediately.
- `renew` extends an existing context lease using backend time.
- Occupied-project conflicts return a stable HTTP `409` error without exposing the owning surface or window id in the response.

### Filesystem browsing

The filesystem directory route supports the custom Web path picker.

- Omitting `path` lists the Web Server process user's home directory.
- Supplied paths must be absolute. Relative paths return `invalid_file_system_path`.
- Responses include the current path, parent path, server-derived breadcrumbs, and all child entries.
- Hidden entries are included. Symbolic links remain visible and preserve their link paths; broken links are reported as unavailable entries.
- Directories sort before files, and the endpoint returns the complete directory without pagination.
- The route intentionally has no configured browse root and can navigate outside home. Deployments must account for the exposed server directory metadata when setting network access to the Web Server.

## Frontend development modes

- `task run:web-backend` starts the Rust HTTP backend on its default port.
- `task run:web-frontend` starts Vite with the fetch contracts transport and expects the backend to run separately.
- `task run:web-proto` starts Vite with the MSW contracts transport and does not require the Rust backend.

The Web production build always selects the fetch transport. Development startup rejects missing or unknown `VITE_ORA_CONTRACT_TRANSPORT` values so the active data source is never implicit.

## Storage Behavior

The current runtime uses a file-backed SQLite database bootstrapped through `ora-db`.

- Data persists across process restarts as long as the same `ORA_DATA_DIR` is reused.
- Readiness depends on successful database bootstrap, repository-pool construction, bootstrap-project reconciliation, and synthetic web work context reconciliation.
- Shared backend failures map into the structured HTTP error envelope using the same public code and message returned by Desktop commands. HTTP alone adds the status code.
