## Purpose

Define the persisted web server runtime contract for Ora's HTTP backend, including SQLite-backed bootstrap, CRUD route exposure, and stable readiness and error behavior.
## Requirements
### Requirement: Web server runtime SHALL bootstrap a file-backed SQLite application state
The system SHALL make `apps/web/server` construct its shared runtime state from a file-backed SQLite database through `ora-db` during startup. The runtime SHALL load a runtime data root and a configured bootstrap project identity from typed bootstrap configuration, SHALL derive the SQLite database path, worktree root, and log file path from that data root, SHALL run database bootstrap and repository-pool construction before marking the server ready, SHALL reconcile the configured project into persistent storage before application state is returned, and SHALL fail startup with a typed bootstrap error when the data root or bootstrap project configuration is invalid or the SQLite bootstrap sequence cannot complete.

#### Scenario: Server starts with a usable database path and a missing configured project
- **WHEN** `ora-web-server` starts with a valid `ORA_DATA_DIR` plus `ORA_PROJECT_NAME` and `ORA_PROJECT_PATH`, and no visible project row exists with that configured name
- **THEN** startup bootstraps SQLite at `<ORA_DATA_DIR>/ora.sqlite3`, creates one project row with the configured name and path, constructs the shared runtime state, and only then reports readiness success

#### Scenario: Server starts with an existing configured project whose stored path drifted
- **WHEN** `ora-web-server` starts with a valid bootstrap configuration and a visible project row already exists for the configured project name but its stored `root_path` differs from `ORA_PROJECT_PATH`
- **THEN** startup updates that existing project row in place to the configured path before the runtime is considered ready

#### Scenario: Server starts with a usable database path and an already reconciled configured project
- **WHEN** `ora-web-server` starts with a valid bootstrap configuration and a visible project row already exists whose name and path match the configured project identity
- **THEN** startup leaves the existing row unchanged, constructs shared repositories and handlers, and reports readiness success

#### Scenario: Bootstrap project configuration is invalid
- **WHEN** `ora-web-server` starts with a blank or missing configured bootstrap project name or path
- **THEN** startup fails with a typed bootstrap error instead of serving requests with an unknown workspace identity

#### Scenario: Database bootstrap fails during startup
- **WHEN** the configured SQLite database cannot be opened, migrated, or pooled during web-server bootstrap
- **THEN** startup fails with a typed bootstrap error instead of serving requests with a partially initialized runtime

### Requirement: Web server runtime SHALL expose HTTP CRUD routes for supported persisted models
The system SHALL expose create, get, list, update, and delete HTTP routes for `project`, `task`, and `session`, and it SHALL expose the backend-managed project-work-context routes required for web runtime ownership. The runtime SHALL NOT expose standalone public worktree CRUD routes. Each supported route SHALL translate transport input into the matching `ora-contracts` request DTO, SHALL delegate to the corresponding `ora-application` handler, and SHALL serialize the returned `ora-contracts` response DTO without adding adapter-local response shapes. Task-create runtime wiring SHALL provide the configured project repository and worktree root needed for backend-owned linked-worktree provisioning.

#### Scenario: Client performs task CRUD over HTTP
- **WHEN** a caller creates, fetches, lists, updates, or deletes a task through the web server
- **THEN** the server delegates to the matching task application handler and returns the matching `ora-contracts` task response payload

#### Scenario: Task creation provisions an internal worktree through runtime wiring
- **WHEN** a caller creates a task through the web server
- **THEN** the runtime-owned task-create dependencies use the configured project repository and the effective worktree root to provision the task's linked worktree before the created task response is returned

#### Scenario: Client attempts standalone worktree CRUD over HTTP
- **WHEN** a caller targets `/api/worktrees` or `/api/worktrees/{worktree_id}`
- **THEN** the runtime does not provide those routes as part of the supported public HTTP API

#### Scenario: Client performs session CRUD over HTTP
- **WHEN** a caller creates, fetches, lists, updates, or deletes a session through the web server
- **THEN** the server delegates to the matching session application handler and returns the matching `ora-contracts` session response payload

#### Scenario: Existing project CRUD uses persisted storage
- **WHEN** a caller creates a project and later fetches or lists projects through the same SQLite-backed runtime
- **THEN** the project routes read and write through the persistent repository-backed application handlers instead of an in-memory bootstrap store

### Requirement: Web server runtime SHALL keep HTTP readiness and error semantics stable across supported model routes
The system SHALL return readiness success only after the database-backed runtime state is fully initialized, and it SHALL map application-layer not-found and repository failures for `project`, `task`, and `session` into stable structured HTTP error responses across the supported route families.

#### Scenario: Resource route requests a missing entity
- **WHEN** any project, task, or session get, update, or delete route delegates to an application handler that returns a not-found outcome
- **THEN** the server responds with an HTTP not-found status and a structured error payload that identifies the missing entity family

#### Scenario: Resource route encounters a repository failure
- **WHEN** any project, task, or session route delegates to an application handler that returns a repository failure
- **THEN** the server responds with an HTTP server-error status and a structured error payload instead of leaking raw infrastructure error formatting

### Requirement: Web server runtime SHALL surface task-provisioning failures as stable HTTP errors
The system SHALL translate task-create failures caused by linked-worktree provisioning or cleanup into stable structured HTTP error responses instead of leaking Git command details or filesystem-specific formatting.

#### Scenario: Linked-worktree provisioning fails for task creation
- **WHEN** the task-create flow encounters a linked-worktree provisioning failure in the web runtime
- **THEN** the server responds with a structured server-error payload that identifies task creation as failed without exposing raw Git command output

### Requirement: Web server runtime SHALL bootstrap into a listening HTTP service
The system SHALL make `ora-web-server` start a real HTTP server process after logging initialization succeeds. The runtime SHALL load a typed server bind configuration from `ORA_HOST` and `ORA_PORT`, SHALL default that bind configuration to `0.0.0.0:32578`, SHALL fail startup on invalid bind configuration, and SHALL continue serving requests until shutdown.

#### Scenario: Server starts with default configuration
- **WHEN** `ora-web-server` starts without overriding its bind-related environment variables
- **THEN** it initializes logging, constructs application state, binds `0.0.0.0:32578`, and begins serving HTTP requests

#### Scenario: Server rejects invalid bind configuration
- **WHEN** `ora-web-server` receives an invalid host or port configuration value during bootstrap
- **THEN** startup fails with a typed bootstrap error instead of silently falling back to an unexpected listener address

### Requirement: Web server runtime SHALL expose operational health endpoints
The system SHALL expose lightweight HTTP endpoints that allow callers to verify process liveness and bootstrap readiness without invoking project application use cases directly. The readiness endpoint SHALL return success only after application-state bootstrap has completed successfully.

#### Scenario: Liveness endpoint is requested
- **WHEN** a caller sends an HTTP request to the configured liveness route
- **THEN** the server returns a successful response that confirms the process is running

#### Scenario: Readiness endpoint is requested after successful bootstrap
- **WHEN** a caller sends an HTTP request to the configured readiness route after application state finishes bootstrapping
- **THEN** the server returns a successful response that confirms the runtime is ready to handle requests

#### Scenario: Readiness endpoint is requested before bootstrap succeeds
- **WHEN** the runtime has not completed application-state bootstrap successfully
- **THEN** the readiness route does not return a success response

### Requirement: Web server runtime SHALL expose HTTP project CRUD routes backed by application handlers
The system SHALL expose HTTP routes for create, get, list, update, and delete project operations, and each route SHALL translate transport input into the corresponding `ora-contracts` request DTO before delegating to the matching `ora-application` project handler.

#### Scenario: Client creates a project over HTTP
- **WHEN** a caller submits a valid create-project HTTP request payload
- **THEN** the server invokes `CreateProjectHandler` and returns a serialized create-project response derived from `ora-contracts`

#### Scenario: Client lists projects over HTTP
- **WHEN** a caller requests the project listing route
- **THEN** the server invokes `ListProjectsHandler` and returns a serialized list-projects response derived from `ora-contracts`

#### Scenario: Client deletes a project over HTTP
- **WHEN** a caller requests deletion for an existing project identifier
- **THEN** the server invokes `DeleteProjectHandler` and returns the delete-project contract response rather than transport-local soft-delete details

### Requirement: Web server runtime SHALL map application failures into stable HTTP responses
The system SHALL centralize transport error mapping for `ora-web-server` so application-layer not-found and repository failure outcomes become stable HTTP error responses instead of leaking internal error formatting directly to callers.

#### Scenario: Requested project does not exist
- **WHEN** a get, update, or delete route delegates to an application handler that returns a not-found outcome
- **THEN** the server responds with an HTTP not-found status and a structured error payload

#### Scenario: Application operation fails internally
- **WHEN** a project route delegates to an application handler that returns an internal repository or bootstrap failure
- **THEN** the server responds with an HTTP server-error status and a structured error payload without exposing transport-irrelevant internals
