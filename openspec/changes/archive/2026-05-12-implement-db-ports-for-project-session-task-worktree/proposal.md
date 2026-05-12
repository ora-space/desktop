## Why

`ora-application` already defines CRUD handlers and repository ports for `project`, `task`, `session`, and `worktree`, but `ora-db` still stops at bootstrap and migration plumbing. Implementing SQLite-backed adapters for those ports is the next step that turns the current handler surface into a usable end-to-end persistence slice for the IDE.

## What Changes

- Add SQLite-backed repository implementations in `ora-db` for the `ProjectRepository`, `TaskRepository`, `SessionRepository`, and `WorktreeRepository` ports defined by `ora-application`.
- Add row-mapping and query code that persists the existing `ora-domain` entities against the current schema, including soft-delete behavior and filtering of deleted rows from visible reads.
- Expose a database composition surface that lets callers construct the repository adapters from an `r2d2`-managed SQLite connection pool without moving orchestration out of `ora-application`.
- Configure pooled SQLite connections for WAL mode, a `busy_timeout` of `5000`, and `synchronous = NORMAL` so the persistence layer uses the intended concurrency and durability profile.
- Add focused integration-style tests in `ora-db` that prove create, get, list, update, and delete flows for all four entity families against SQLite.
- Update documentation that currently describes `ora-db` as the future home of these ports so it reflects the new implemented adapter surface.

## Capabilities

### New Capabilities

- `database-repositories`: SQLite-backed implementations of the application repository ports for `project`, `task`, `session`, and `worktree`.

### Modified Capabilities

None.

## Impact

- Affected code: `crates/db`, `crates/application` integration points, and applicable files in `docs/`.
- Affected APIs: new public `ora-db` exports for constructing and using repository adapters that implement `ora-application` ports.
- Dependencies: adds `r2d2`-based connection pooling on top of the existing SQLite stack used by `ora-db`.
- Systems: unblocks wiring application handlers to real persistence in runtime adapters without changing handler contracts or domain models.
