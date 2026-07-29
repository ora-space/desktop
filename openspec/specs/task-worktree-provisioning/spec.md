## Purpose

Define the backend-owned linked-worktree lifecycle that task creation and deletion must enforce.

## Requirements

### Requirement: Task creation SHALL provision exactly one internal linked worktree
The system SHALL treat linked worktree provisioning as part of task creation. When a new task is created, the backend SHALL resolve the selected local base branch to its immutable commit ID, SHALL derive a task-owned branch name and worktree root from the generated task identifier, SHALL create one linked Git worktree at that commit from the configured project repository, SHALL persist one worktree record for that checkout, and SHALL persist the task with the new internal `worktree_id`.

#### Scenario: Creating a task provisions and links a worktree
- **WHEN** the backend handles a valid task creation request for the configured project
- **THEN** it creates one linked Git worktree, persists one worktree record, and stores the task-to-worktree linkage internally without exposing the internal `worktree_id` in the public task payload

#### Scenario: Branch and path are derived from the generated task identity
- **WHEN** the backend generates a new task identifier during task creation
- **THEN** it derives the Git branch name from a short task-id prefix and the worktree directory from the full task identifier under the configured worktree root

#### Scenario: Selected base branch fixes the worktree start commit
- **WHEN** a caller selects an existing local base branch for a new task
- **THEN** the backend resolves that branch to a commit ID before creating the task branch
- **THEN** the task worktree starts at that resolved commit even if another branch is currently checked out

#### Scenario: Ora branch labels use task titles
- **WHEN** the frontend lists a managed `ora/<prefix>` branch
- **THEN** it displays the owning task title while retaining the real branch name for Git requests

#### Scenario: Short branch-prefix collisions regenerate the task identity
- **WHEN** the generated short task-id prefix is already used by a task worktree directory or a local `ora/<prefix>` branch
- **THEN** the backend generates another task identifier before provisioning, because the collision applies to the shortened branch name rather than the full-identifier worktree directory

#### Scenario: Orphaned task branches still reserve their prefix
- **WHEN** a task worktree directory has been removed but its local `ora/<prefix>` branch remains
- **THEN** the backend treats that prefix as unavailable and generates another task identifier

### Requirement: Task creation SHALL keep Git and persistence state aligned
The system SHALL avoid exposing partially created task workspaces. If linked-worktree provisioning fails, the backend SHALL return a failure without persisting task or worktree rows. If persistence fails after the Git worktree is created, the backend SHALL attempt compensating cleanup of that linked worktree before returning a failure.

#### Scenario: Git provisioning fails before persistence
- **WHEN** linked-worktree creation fails for a new task
- **THEN** the backend returns a task-creation failure and no task or worktree record is persisted

#### Scenario: Task persistence fails after Git worktree creation
- **WHEN** the backend has already created the linked worktree but later fails to persist the worktree or task record
- **THEN** it attempts to delete the created linked worktree before returning a failure outcome

### Requirement: Task deletion SHALL remove the task-owned linked worktree with force mode
The system SHALL treat the linked worktree as backend-owned task state. When a task is deleted, the backend SHALL remove the linked worktree associated with that task before finalizing deletion, and it SHALL use force mode for the first version so dirty worktrees do not block cleanup.

#### Scenario: Deleting a task removes its linked worktree
- **WHEN** the backend deletes a task that has an associated linked worktree
- **THEN** it removes that linked worktree and completes task deletion

#### Scenario: Dirty linked worktree does not block task deletion
- **WHEN** the linked worktree associated with a task contains uncommitted changes during task deletion
- **THEN** the backend uses force-mode worktree removal so cleanup still proceeds
