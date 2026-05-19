## Purpose

Define the SQLite-backed repository adapter surface that `ora-db` provides for the core `ora-application` repository ports.

## Requirements

### Requirement: Ora DB SHALL implement SQLite-backed repositories for the core application ports
The system SHALL provide SQLite-backed implementations in `ora-db` for `ora-application`'s `ProjectRepository`, `TaskRepository`, `SessionRepository`, and `WorktreeRepository` traits without changing the handler-owned port definitions.

#### Scenario: Runtime composes application handlers with SQLite repositories
- **WHEN** a composition root bootstraps a `Database` from `ora-db`
- **THEN** it can construct repository adapters from `ora-db` that satisfy the corresponding `ora-application` repository traits for `project`, `task`, `session`, and `worktree`

### Requirement: Ora DB SHALL use pooled SQLite connections with the required runtime settings
The system SHALL construct the new SQLite repository adapters from an `r2d2` connection pool, and each pooled SQLite connection SHALL be configured with `journal_mode = WAL`, `busy_timeout = 5000`, and `synchronous = NORMAL`.

#### Scenario: Repository pool is created for a file-backed database
- **WHEN** a composition root creates the `ora-db` repository adapter pool for a file-backed SQLite database
- **THEN** the pool uses `r2d2` and initializes each SQLite connection with WAL journaling, a `busy_timeout` of `5000`, and `synchronous = NORMAL`

### Requirement: Repository reads SHALL hide soft-deleted rows
The system SHALL treat `is_deleted = 1` rows as deleted implementation detail in `ora-db`, so visible find and list operations only return non-deleted domain entities while delete operations update the soft-delete state.

#### Scenario: Soft-deleted row is queried by identifier
- **WHEN** a caller asks an `ora-db` repository to find a `project`, `task`, `session`, or `worktree` whose row has `is_deleted = 1`
- **THEN** the repository returns `None` instead of exposing the deleted row to the application layer

#### Scenario: Visible rows are listed
- **WHEN** a caller lists `project`, `task`, `session`, or `worktree` entities through an `ora-db` repository
- **THEN** the repository returns only rows whose `is_deleted` flag is not set

#### Scenario: Visible row is soft-deleted
- **WHEN** a caller invokes a repository soft-delete operation for an existing visible entity
- **THEN** `ora-db` updates the row `is_deleted` flag and `updated_at` timestamp and reports that one visible entity was deleted

### Requirement: Project repositories SHALL support visible lookup by project name
The system SHALL allow `ora-db` project repositories to load one visible `Project` by its exact persisted `name` so bootstrap flows can reconcile a configured workspace identity without listing the full project table. Name-based lookup SHALL ignore soft-deleted rows the same way identifier-based reads do.

#### Scenario: Visible project is queried by exact name
- **WHEN** a caller asks an `ora-db` project repository to find a project by a name stored on one visible row
- **THEN** the repository returns that full `Project` snapshot, including its existing identifier, root path, and audit fields

#### Scenario: Soft-deleted project shares the queried name
- **WHEN** a caller asks an `ora-db` project repository to find a project by name and only soft-deleted rows match that name
- **THEN** the repository returns `None` instead of exposing deleted project rows to bootstrap or application code

#### Scenario: No visible project matches the queried name
- **WHEN** a caller asks an `ora-db` project repository to find a project by name that does not exist among visible rows
- **THEN** the repository returns `None`

### Requirement: Repository adapters SHALL map persisted rows to existing domain models
The system SHALL persist and load `ora-domain` `Project`, `Task`, `Session`, and `Worktree` values by mapping SQLite columns to the current domain shapes, including audit fields and enum-backed integer columns already defined by the domain layer.

#### Scenario: Task row is loaded from SQLite
- **WHEN** `ora-db` reads a `tasks` row
- **THEN** it converts the persisted `status` integer into `ora_domain::TaskStatus`, maps `worktree_id` into an optional `WorktreeId`, and returns a full `ora_domain::Task` with audit fields populated from the row

#### Scenario: Session row is loaded from SQLite
- **WHEN** `ora-db` reads a `sessions` row
- **THEN** it converts the persisted `status` integer into `ora_domain::SessionStatus`, preserves the optional `agent_session_id`, and returns a full `ora_domain::Session` with audit fields populated from the row

#### Scenario: Worktree row is loaded from SQLite
- **WHEN** `ora-db` reads a `worktrees` row
- **THEN** it converts the persisted `is_active` integer into `ora_domain::WorktreeActivity`, preserves the optional `branch_name`, and returns a full `ora_domain::Worktree` with audit fields populated from the row

### Requirement: Repository implementations SHALL preserve CRUD replacement semantics
The system SHALL make the `create_*` and `update_*` port operations behave as full domain snapshot persistence operations so the returned entity matches the state stored in SQLite after the write succeeds.

#### Scenario: Application creates an entity through a repository
- **WHEN** `ora-application` passes a newly built `Project`, `Task`, `Session`, or `Worktree` into the matching `ora-db` repository `create_*` method
- **THEN** the repository stores that snapshot in SQLite and returns the stored domain entity without adding transport-specific data

#### Scenario: Application updates an entity through a repository
- **WHEN** `ora-application` passes a replacement `Project`, `Task`, `Session`, or `Worktree` into the matching `ora-db` repository `update_*` method
- **THEN** the repository updates the persisted row to match the provided snapshot and returns the stored domain entity

### Requirement: Ora DB SHALL surface repository failures through application-owned error types
The system SHALL translate SQLite execution, query, and row-mapping failures into the matching `ProjectRepositoryError`, `TaskRepositoryError`, `SessionRepositoryError`, or `WorktreeRepositoryError` values expected by `ora-application`.

#### Scenario: SQLite write fails during repository operation
- **WHEN** a SQLite statement execution fails while `ora-db` performs a repository create, update, list, find, or delete operation
- **THEN** the repository returns the matching application-owned repository error instead of exposing raw `rusqlite` errors across the boundary

### Requirement: Session repository mappings SHALL preserve typed agent identifiers
The SQLite session repository SHALL map the persisted `sessions.agent_id` text column to and from `ora_domain::AgentId` when creating, updating, finding, and listing `Session` values. Repository callers SHALL receive typed session agent identities without any database schema change.

#### Scenario: Session row is loaded from SQLite with an agent identifier
- **WHEN** `ora-db` reads a visible `sessions` row
- **THEN** it converts the persisted `agent_id` text into `ora_domain::AgentId`, preserves the existing `agent_session_id` and status semantics, and returns a full `ora_domain::Session`

#### Scenario: Typed session is written back to SQLite
- **WHEN** `ora-db` creates or updates a `Session` whose `agent_id` is an `ora_domain::AgentId`
- **THEN** the repository persists the same underlying string value into the `sessions.agent_id` column without requiring a schema migration or adapter-local shadow field
