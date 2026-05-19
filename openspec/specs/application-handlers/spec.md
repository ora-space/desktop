## Purpose

Define transport-agnostic application handlers and handler-owned ports for the first `project` CRUD vertical slice.

## Requirements

### Requirement: Project CRUD handlers SHALL be transport-agnostic
The system SHALL define `CreateProjectHandler`, `GetProjectHandler`, `ListProjectsHandler`, `UpdateProjectHandler`, and `DeleteProjectHandler` in `ora-application` as transport-agnostic entry points, and it SHALL define matching `CreateTaskHandler`, `GetTaskHandler`, `ListTasksHandler`, `UpdateTaskHandler`, `DeleteTaskHandler`, `CreateWorktreeHandler`, `GetWorktreeHandler`, `ListWorktreesHandler`, `UpdateWorktreeHandler`, `DeleteWorktreeHandler`, `CreateSessionHandler`, `GetSessionHandler`, `ListSessionsHandler`, `UpdateSessionHandler`, and `DeleteSessionHandler` for the corresponding entity slices. Each handler SHALL accept exactly one request contract type and SHALL return exactly one response contract type or an application error without referencing HTTP, Tauri, or database-specific types.

#### Scenario: HTTP adapter invokes a handler
- **WHEN** an HTTP route receives a `project`, `task`, `worktree`, or `session` CRUD request
- **THEN** it can deserialize transport input into one `ora-contracts` request value, call the corresponding `ora-application` handler, and serialize the returned contract response or application error without additional use-case orchestration in the adapter

#### Scenario: Tauri adapter invokes a handler
- **WHEN** a Tauri command needs to perform a `project`, `task`, `worktree`, or `session` CRUD action
- **THEN** it can translate command input into one `ora-contracts` request value and delegate the use case to the same `ora-application` handler API used by other adapters

### Requirement: Application handlers SHALL own project repository ports
The system SHALL define the repository and supporting dependency traits required by the `project`, `task`, `worktree`, and `session` CRUD handlers inside `ora-application`, and handlers SHALL depend on those ports rather than concrete persistence implementations.

#### Scenario: Unit test executes a handler
- **WHEN** a test constructs a `project`, `task`, `worktree`, or `session` handler with in-memory fake dependencies
- **THEN** the handler can execute the full use case without a database, HTTP server, or Tauri runtime

#### Scenario: Persistence adapter integrates later
- **WHEN** `ora-db` implements persistence for `project`, `task`, `worktree`, and `session` use cases
- **THEN** it implements `ora-application` ports instead of changing handler signatures or moving orchestration logic into the database crate

### Requirement: DeleteProjectHandler SHALL preserve CRUD semantics externally
The system SHALL expose deletion through `DeleteProjectHandler`, `DeleteTaskHandler`, `DeleteWorktreeHandler`, and `DeleteSessionHandler` as normal CRUD delete use cases even if the underlying repository implementation performs a soft delete internally.

#### Scenario: Adapter requests entity deletion
- **WHEN** an adapter invokes one of the delete handlers for `project`, `task`, `worktree`, or `session`
- **THEN** it interacts with a delete-oriented request and response contract rather than transport-visible soft-delete or archive semantics

### Requirement: Project CRUD handlers SHALL emit structured operational events
The system SHALL require the `project`, `task`, `worktree`, and `session` CRUD handlers to emit structured operational logs from `ora-application` without introducing transport-specific concerns. These events SHALL use the shared JSON logging envelope and SHALL include business context such as the use-case operation name and relevant entity identifiers when available.

#### Scenario: Handler completes a use case successfully
- **WHEN** a CRUD handler for `project`, `task`, `worktree`, or `session` completes successfully
- **THEN** `ora-application` emits an informational event that identifies the operation and includes the relevant entity identifier when that identifier is available for the use case

#### Scenario: Handler encounters an application-layer failure
- **WHEN** a CRUD handler for `project`, `task`, `worktree`, or `session` returns a not-found or repository failure outcome
- **THEN** `ora-application` emits an error event that records the operation context and failure details without requiring an HTTP or Tauri adapter to add the log entry itself

### Requirement: CreateTaskHandler SHALL orchestrate task-owned worktree provisioning
The system SHALL require `CreateTaskHandler` to generate the task identifier, derive the task-owned branch name and worktree root, delegate linked-worktree provisioning through an application-owned port, persist the worktree record, and persist the task record with the resulting `worktree_id`. This orchestration SHALL remain transport-agnostic and SHALL NOT move Git-specific types into request or response contracts.

#### Scenario: Handler creates a task successfully
- **WHEN** `CreateTaskHandler` receives a valid task creation request
- **THEN** it orchestrates linked-worktree provisioning and persistence before returning the shared created-task response

#### Scenario: Handler unit test replaces Git provisioning
- **WHEN** a unit test constructs `CreateTaskHandler` with fake repositories and a fake provisioning dependency
- **THEN** the complete task-create flow can be exercised without a real Git repository or filesystem side effects

### Requirement: CreateTaskHandler SHALL treat provisioning failures as stable application failures
The system SHALL require `CreateTaskHandler` to convert provisioning and compensating-cleanup failures into stable application errors and structured logs, and it SHALL avoid persisting partial task state when those failures occur.

#### Scenario: Provisioning dependency fails before persistence
- **WHEN** the task worktree provisioning port returns a failure before any task row is created
- **THEN** `CreateTaskHandler` returns an application failure and logs the create-task failure context

#### Scenario: Compensation is needed after a persistence failure
- **WHEN** worktree or task persistence fails after linked-worktree creation succeeded
- **THEN** `CreateTaskHandler` attempts cleanup through the same provisioning port and returns a stable application failure outcome

### Requirement: Task, worktree, and session handlers SHALL defer relationship validation
The system SHALL treat `project_id`, `task_id`, and `worktree_id` fields in the first `task`, `worktree`, and `session` handler slice as pass-through business identifiers, and create or update handlers MUST NOT perform extra cross-entity existence checks before delegating to their repositories.

#### Scenario: Task handler receives a referenced project identifier
- **WHEN** `CreateTaskHandler` or `UpdateTaskHandler` receives a `project_id` and optional `worktree_id`
- **THEN** it constructs the domain task snapshot and delegates persistence without first loading the referenced `project` or `worktree`

#### Scenario: Session handler receives a referenced task identifier
- **WHEN** `CreateSessionHandler`, `UpdateSessionHandler`, `CreateWorktreeHandler`, or `UpdateWorktreeHandler` receives a `task_id`
- **THEN** it delegates the request without first validating that the related task already exists
