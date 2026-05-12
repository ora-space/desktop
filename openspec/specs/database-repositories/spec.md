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
