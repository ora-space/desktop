## Purpose

Define the transport-neutral, serialization-friendly contract surface used by adapters and frontend generation for the first `project` vertical slice.

## Requirements

### Requirement: Project contracts SHALL define the first frontend-facing CRUD protocol
The system SHALL define request and response DTOs for `CreateProject`, `GetProject`, `ListProjects`, `UpdateProject`, and `DeleteProject`, and it SHALL define matching DTOs for `CreateTask`, `GetTask`, `ListTasks`, `UpdateTask`, `DeleteTask`, `CreateWorktree`, `GetWorktree`, `ListWorktrees`, `UpdateWorktree`, `DeleteWorktree`, `CreateSession`, `GetSession`, `ListSessions`, `UpdateSession`, and `DeleteSession` in `ora-contracts`. These DTOs SHALL be the transport-neutral contract surface used by adapters and by frontend type generation.

#### Scenario: Adapter needs a request payload type
- **WHEN** an HTTP or Tauri adapter accepts input for a `project`, `task`, `worktree`, or `session` CRUD action
- **THEN** it uses the corresponding `ora-contracts` request DTO instead of transport-local ad hoc structs

#### Scenario: Frontend types are generated
- **WHEN** the repository generates frontend-consumable types from Rust contracts
- **THEN** the generated types come from `ora-contracts` rather than directly from domain entities or adapter-specific payload structs

### Requirement: Project view contracts SHALL expose a single public project shape
The system SHALL expose a single shared public view model for each first-slice entity in `ora-contracts`. `ora_contracts::Project` SHALL include `id`, `name`, and `root_path`; `ora_contracts::Task` SHALL include `id`, `project_id`, `title`, `status`, and `worktree_id`; `ora_contracts::Worktree` SHALL include `id`, `task_id`, `branch_name`, and `activity`; and `ora_contracts::Session` SHALL include `id`, `task_id`, `agent_id`, `agent_session_id`, and `status`.

#### Scenario: Handler returns an entity to an adapter
- **WHEN** a create, get, list, or update use case returns `project`, `task`, `worktree`, or `session` data
- **THEN** the response uses the corresponding shared `ora-contracts` view shape instead of separate summary and detail variants

#### Scenario: Caller inspects public payload fields
- **WHEN** an adapter or generated frontend type consumes one of the first-slice public view models
- **THEN** it receives only the documented business fields and does not receive `created_at`, `updated_at`, `is_deleted`, or other internal audit fields

### Requirement: Contract types SHALL remain serialization-friendly and domain-decoupled
The system SHALL keep `ora-contracts` types suitable for serialization and frontend generation, and SHALL require `ora-application` to map domain entities into those contracts rather than exposing raw domain models directly for `project`, `task`, `worktree`, or `session` payloads.

#### Scenario: Domain model evolves internally
- **WHEN** the domain layer adds internal fields or invariants that are not part of the app-facing protocol
- **THEN** adapters and generated frontend types remain bound to `ora-contracts` shapes instead of inheriting those internal domain details automatically

### Requirement: Task contracts SHALL keep worktree ownership internal to the backend
The system SHALL define `CreateTaskRequest` so callers provide task identity inputs only: `project_id`, `title`, and `status`. The create-task contract SHALL NOT accept a caller-supplied `worktree_id`, because task worktree assignment is an internal backend responsibility. The shared `Task` view and `UpdateTaskRequest` SHALL NOT expose backend-assigned `worktree_id` values to callers.

#### Scenario: Adapter submits a task creation request
- **WHEN** an HTTP or Tauri adapter constructs a `CreateTaskRequest`
- **THEN** the request shape excludes `worktree_id` and includes only the project, title, and status fields required to create the task

#### Scenario: Caller receives a task payload
- **WHEN** a create, get, list, or update task use case returns successfully
- **THEN** the shared `Task` response payload excludes backend-assigned `worktree_id`

#### Scenario: Adapter submits a task update request
- **WHEN** an HTTP or Tauri adapter constructs an `UpdateTaskRequest`
- **THEN** the request shape excludes `worktree_id` and includes only the public task fields callers can update
