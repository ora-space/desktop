# Web Server Runtime

`apps/web/server` is Ora's HTTP backend runtime.

## Purpose

- It boots shared structured logging through `ora-logging` and registers the Gitlancer command-logging bridge.
- It exposes health endpoints for process liveness and runtime readiness.
- It serves persisted HTTP operations for Project, Task, Session, Skill, Agent, and Git identity through the shared `ora-backend` composition.
- It provisions task-owned linked worktrees during creation and leaves Git untouched during deletion.
- It streams ACP load replay and prompt updates as bounded NDJSON responses.
- It provides read-only server filesystem listings for the Web platform path picker.
- It owns the project work context routes, which are outside `ora-backend`.

## Data root configuration

The web server reads one runtime data root:

- `ORA_DATA_DIR`: root directory for runtime state. Default: `.`, resolved to an absolute path against the process working directory so Git commands running elsewhere still resolve it correctly. A blank value fails startup.

Every other runtime path is derived from it — there is no separate variable for any of them:

- SQLite database: `<ORA_DATA_DIR>/ora.sqlite3`
- Worktree creation root: `<ORA_DATA_DIR>/worktrees`
- Log file: `<ORA_DATA_DIR>/logs/ora.log`

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
- `DELETE /api/sessions/{sessionId}`

### agentRuntime

- `GET /api/agent-models`

### skill

- `POST /api/skills`
- `GET /api/skills`
- `GET /api/skills/{skillId}`
- `PUT /api/skills/{skillId}`
- `DELETE /api/skills/{skillId}`

### agent

- `POST /api/agents`
- `GET /api/agents`
- `GET /api/agents/{agentId}`
- `PUT /api/agents/{agentId}`
- `DELETE /api/agents/{agentId}`

### fileSystem

- `GET /api/file-system/directory?path={absolute_path}`

### gitIdentity

- `GET /api/git/identity`

Each route translates transport input into the matching `ora-contracts` request DTO, delegates to the shared backend, and serializes the returned contract response without adding adapter-local response shapes.

Task payloads do not expose backend-owned worktree identifiers, and the runtime exposes no standalone public worktree endpoints — `/api/worktrees` and `/api/worktrees/{worktreeId}` are not part of the API.

`GET /api/git/identity` returns the host's Git identity for the sidebar profile: the global Git config first, falling back to the authenticated GitHub CLI account when Git has no name configured.

### Agent runtime

Backend construction immediately attempts one supervised `acp` child per supported CLI, rooted at the user's home directory. Executable resolution is platform-specific: on Unix each CLI is read from its fixed per-user directory (`<home>/.opencode/bin/opencode`, `<home>/.nga/bin/nga`, `<home>/.codeagentcli/bin/codeagentcli`); on Windows it is resolved from `PATH` through `where.exe` on every retry generation.

Each independent supervisor performs `initialize` once per process generation and retries failures without blocking healthy CLIs or non-agent APIs. Session create calls `session/new` on the connection selected by `agentCli` and returns the latest available-command catalog announced during setup. Updates emitted before the response reveals the private provider session id are temporarily buffered, then attached to the matching session route. Load calls `session/load` using that private id and the Task worktree `cwd`; the public Session payload never exposes it. `GET /api/agent-models` concurrently runs each CLI's bounded `models` discovery command and returns only successful groups.

Prompt requests carry an ordered ACP content-block list and may combine text, images, audio, and resources. The serialized prompt is limited to 16 MiB. Load and prompt responses use `application/x-ndjson`; each line is one complete frame. Data and control paths are separate, session-update queues are bounded at 256 items, frames are limited to 8 MiB, and overflow terminates the operation rather than dropping updates silently. See [ACP Agent Runtime](agent-runtime.md).

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

Shared backend failures use the same public code and message that Desktop commands return; HTTP alone adds the status code.

## Frontend development modes

- `task run:web-backend` starts the Rust HTTP backend on its default port.
- `task run:web-frontend` starts Vite with the fetch contracts transport and expects the backend to run separately.

The Web frontend always uses the fetch contracts transport and talks to the Rust HTTP backend, in both development and production builds.

## Storage behavior

The runtime uses a file-backed SQLite database bootstrapped through `ora-db`.

- Data persists across process restarts as long as the same `ORA_DATA_DIR` is reused.
- Readiness depends on successful database bootstrap, repository-pool construction, bootstrap-project reconciliation, and synthetic web work context reconciliation.
