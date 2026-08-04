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
- Skill packages root: `<ORA_DATA_DIR>/atoms/skills`
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
- Agent Session startup resolves Task → Worktree → branch and then asks Git for the authoritative linked-worktree path supplied as the ACP session `cwd`.
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
- `GET /api/agent-models`
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
- `POST /api/skill-imports?mode={folder|archive}`
- `GET /api/skill-imports/{session_id}`
- `POST /api/skill-imports/{session_id}/commit`
- `DELETE /api/skill-imports/{session_id}`
- `POST /api/agents`
- `GET /api/agents`
- `GET /api/agents/{agent_id}`
- `PUT /api/agents/{agent_id}`
- `DELETE /api/agents/{agent_id}`
- `GET /api/file-system/directory?path={absolute_path}`

Request and response payloads use `ora-contracts` DTO shapes, so transport behavior stays aligned with the shared application contract.
Task payloads do not expose backend-owned worktree identifiers, and the runtime does not expose standalone public worktree CRUD endpoints.

Backend construction immediately attempts `<home>/.opencode/bin/opencode acp`, `<home>/.nga/bin/nga acp`, and `<home>/.codeagentcli/bin/codeagentcli acp` children rooted at the user's home directory. Each independent supervisor performs `initialize` once per process generation and retries failures without blocking healthy CLIs or non-agent APIs. Session create calls `session/new` on the connection selected by `agentCli`; load calls `session/load` using the private provider session id and the Task worktree `cwd`. The public Session payload never exposes that id. `GET /api/agent-models` concurrently runs each CLI's bounded `models` discovery command and returns only successful groups.

Load and prompt responses use `application/x-ndjson`. Each line is one complete frame. Data and control paths are separate, session-update queues are bounded at 256 items, frames are limited to 8 MiB, and overflow terminates the operation rather than dropping updates silently.

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

### Skill import sessions

The import routes implement the two-phase `prepare -> preview -> commit` model with one logical
source per session (a folder tree or a single supported archive: `.zip`, `.skill`, `.tar.gz`,
`.tgz`).

- `POST /api/skill-imports?mode=folder` receives a multipart body whose file parts carry
  validated relative paths (`webkitRelativePath`-style filenames) and returns a prepared session
  preview without touching formal storage.
- `POST /api/skill-imports?mode=archive` receives exactly one file part and streams it to OS
  temporary storage before preparation.
- `GET /api/skill-imports/{session_id}` returns the session, its commit progress, and completed
  per-item results.
- `POST /api/skill-imports/{session_id}/commit` validates and freezes the conflict decisions and
  returns `202 Accepted`; the commit continues as a background task that survives request drops.
- `DELETE /api/skill-imports/{session_id}` cancels a prepared session only; committing sessions
  reject cancellation.

Preparation validates archive signatures, enforces zip-slip and portable case-conflict rules,
applies capacity and expansion-ratio budgets, and parses every `SKILL.md` manifest. Preparation
failures and per-candidate outcomes use the stable import error codes from `ora-backend`. The web
adapter never buffers upload bytes in memory and streams them to disk under a strict size budget;
the multipart body ceiling is raised to just above 200 MiB (axum's default is 2 MiB) while the
handler still enforces the exact 200 MiB folder / 50 MiB archive file budgets.

### Skill storage and startup coordination

Every visible skill owns a database record plus `<ORA_DATA_DIR>/atoms/skills/<name>/SKILL.md`.
At startup, `ora-backend` reconciles interrupted skill transactions from journal markers, cleans
orphan formal directories, and blocks readiness when a visible record lacks its formal directory
or root manifest.

## Frontend development modes

- `task run:web-backend` starts the Rust HTTP backend on its default port.
- `task run:web-frontend` starts Vite with the fetch contracts transport and expects the backend to run separately.

The Web frontend always uses the fetch contracts transport and talks to the Rust HTTP backend, in both development and production builds.

## Storage Behavior

The current runtime uses a file-backed SQLite database bootstrapped through `ora-db`.

- Data persists across process restarts as long as the same `ORA_DATA_DIR` is reused.
- Readiness depends on successful database bootstrap, repository-pool construction, bootstrap-project reconciliation, and synthetic web work context reconciliation.
- Shared backend failures map into the structured HTTP error envelope using the same public code and message returned by Desktop commands. HTTP alone adds the status code.
