## 1. Expand the contract surface

- [x] 1.1 Add `task` contract DTOs and shared public view types in `crates/contracts/src/task.rs`, then re-export them from `crates/contracts/src/lib.rs`.
- [x] 1.2 Add `worktree` contract DTOs and shared public view types in `crates/contracts/src/worktree.rs`, then re-export them from `crates/contracts/src/lib.rs`.
- [x] 1.3 Add `session` contract DTOs and shared public view types in `crates/contracts/src/session.rs`, then re-export them from `crates/contracts/src/lib.rs`.
- [x] 1.4 Add serialization-focused contract tests that cover the new `task`, `worktree`, and `session` request, response, and shared view payloads.

## 2. Add task handlers

- [x] 2.1 Create the `crates/application/src/task/` module with mapper and ports definitions for task CRUD operations, including ID generation and repository traits owned by `ora-application`.
- [x] 2.2 Implement `CreateTaskHandler`, `GetTaskHandler`, `ListTasksHandler`, `UpdateTaskHandler`, and `DeleteTaskHandler` by following the existing `project` handler pattern and preserving soft-delete semantics externally.
- [x] 2.3 Extend `ApplicationError` and task-specific repository error mapping so task handlers return stable not-found and repository failure outcomes.
- [x] 2.4 Add task handler unit tests with in-memory fakes that cover success paths, representative failures, and structured logging behavior.

## 3. Add worktree handlers

- [x] 3.1 Create the `crates/application/src/worktree/` module with mapper and ports definitions for worktree CRUD operations, including ID generation and repository traits owned by `ora-application`.
- [x] 3.2 Implement `CreateWorktreeHandler`, `GetWorktreeHandler`, `ListWorktreesHandler`, `UpdateWorktreeHandler`, and `DeleteWorktreeHandler` by following the existing `project` handler pattern and preserving soft-delete semantics externally.
- [x] 3.3 Extend `ApplicationError` and worktree-specific repository error mapping so worktree handlers return stable not-found and repository failure outcomes.
- [x] 3.4 Add worktree handler unit tests with in-memory fakes that cover success paths, representative failures, and structured logging behavior.

## 4. Add session handlers

- [x] 4.1 Create the `crates/application/src/session/` module with mapper and ports definitions for session CRUD operations, including ID generation and repository traits owned by `ora-application`.
- [x] 4.2 Implement `CreateSessionHandler`, `GetSessionHandler`, `ListSessionsHandler`, `UpdateSessionHandler`, and `DeleteSessionHandler` by following the existing `project` handler pattern and preserving soft-delete semantics externally.
- [x] 4.3 Extend `ApplicationError` and session-specific repository error mapping so session handlers return stable not-found and repository failure outcomes.
- [x] 4.4 Add session handler unit tests with in-memory fakes that cover success paths, representative failures, and structured logging behavior.

## 5. Wire exports and verify the slice

- [x] 5.1 Export the new `task`, `worktree`, and `session` application modules from `crates/application/src/lib.rs` so adapters can consume the new handlers and ports.
- [x] 5.2 Run `cargo fmt --all` and `task test`, then fix any compile, test, or style regressions introduced by the new vertical slices.
