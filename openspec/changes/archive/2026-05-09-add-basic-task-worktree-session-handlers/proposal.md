## Why

The repository already has domain models for tasks, worktrees, and sessions, but the application layer only exposes the first `project` CRUD slice. Adding the same minimal handler surface for the remaining core entities lets adapters and future UI work evolve against a consistent contract-first API instead of ad hoc one-off entry points.

## What Changes

- Add transport-agnostic CRUD handlers for `task`, `worktree`, and `session` in `ora-application`, following the existing `project` handler pattern.
- Add serialization-friendly request, response, and shared view contracts for `task`, `worktree`, and `session` in `ora-contracts`.
- Define handler-owned repository and supporting dependency traits for each new vertical slice so the new handlers remain unit-testable without database or adapter runtimes.
- Keep the first slice intentionally simple by not enforcing cross-model relationship validation such as `task` to `project`, `task` to `worktree`, or `session` to `task` consistency beyond field-level typing.
- Extend application-layer tests so each new handler family has baseline coverage for create, get, list, update, and delete behavior plus representative error paths.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `application-handlers`: Expand the handler requirements from the initial `project` slice to include matching CRUD handlers, ports, and logging behavior for `task`, `worktree`, and `session`.
- `app-contracts`: Expand the transport-neutral contract surface from `project` CRUD only to include DTOs and shared public views for `task`, `worktree`, and `session`.

## Impact

- Affected code: `crates/application`, `crates/contracts`, and their tests.
- Affected APIs: public `ora-application` handler exports and public `ora-contracts` DTO exports.
- Dependencies: no new external dependencies are expected.
- Systems: unblocks future adapters and frontend generation work for the remaining schema-backed entities without coupling those additions to database relationship rules.
