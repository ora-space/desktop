# Web Server Runtime

`apps/web/server` is the first HTTP backend runtime for Ora.

## Purpose

- It boots shared structured logging through `ora-logging`.
- It exposes health endpoints for process liveness and runtime readiness.
- It serves persisted HTTP CRUD routes for `project`, `task`, and `session` by delegating to `ora-application`.
- It provisions and cleans up task-owned linked worktrees as internal backend state during task lifecycle flows.

## Database Configuration

The web server reads its runtime data root from:

- `ORA_DATA_DIR`: root directory for runtime state. Default: `.`

Startup bootstraps the database through `ora-db`, applies the active migration catalog, and constructs the shared repository pool before the runtime is marked ready.

- SQLite database path: `<ORA_DATA_DIR>/ora.sqlite3`
- Worktree root: `<ORA_DATA_DIR>/worktrees`
- Log file: `<ORA_DATA_DIR>/logs/ora.log`

## Project Configuration

The web server also requires a bootstrap project identity:

- `ORA_PROJECT_NAME`: persisted workspace project name. Required.
- `ORA_PROJECT_PATH`: persisted workspace root path. Required.

Startup reconciles this configured project into the `projects` table before the runtime is marked ready.

- If no visible project exists with the configured name, startup creates one row.
- If a visible project exists with the configured name but a different stored path, startup updates that row in place.
- If both the configured name and path already match, startup leaves the row unchanged.
- If `ORA_WORK_DIR` is unset, startup uses a `worktrees/` directory next to the configured SQLite database file.
- Task creation provisions linked worktrees under `ORA_WORK_DIR/<full-task-id>`.
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
- `PUT /api/sessions/{session_id}`
- `DELETE /api/sessions/{session_id}`

Request and response payloads use `ora-contracts` DTO shapes, so transport behavior stays aligned with the shared application contract.
Task payloads do not expose backend-owned worktree identifiers, and the runtime does not expose standalone public worktree CRUD endpoints.

The project work context routes provide the current backend-managed project selection surface.

- `open` creates or switches one `(surface, window_id)` context into a project and refreshes its lease immediately.
- `renew` extends an existing context lease using backend time.
- Occupied-project conflicts return a stable HTTP `409` error without exposing the owning surface or window id in the response.

## Storage Behavior

The current runtime uses a file-backed SQLite database bootstrapped through `ora-db`.

- Data persists across process restarts as long as the same `ORA_DATA_DIR` is reused.
- Readiness depends on successful database bootstrap, repository-pool construction, bootstrap-project reconciliation, and synthetic web work context reconciliation.
- Application-layer failures still map into the shared structured HTTP error envelope across the supported route families.
