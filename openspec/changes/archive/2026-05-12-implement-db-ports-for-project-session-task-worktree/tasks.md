## 1. Prepare the db adapter surface

- [x] 1.1 Add the `ora-application`, `ora-domain`, and `r2d2`-related crate dependencies required for repository trait implementations, pooling, and domain mapping in `crates/db/Cargo.toml`.
- [x] 1.2 Create the internal `ora-db` module structure for repository adapters and shared SQLite helpers, including an `r2d2` pool factory for SQLite connections.
- [x] 1.3 Export the new repository adapter types from `crates/db/src/lib.rs` so runtime composition code can construct them from `ora-db`.
- [x] 1.4 Configure pooled SQLite connections to use WAL mode, `busy_timeout = 5000`, and `synchronous = NORMAL`, and cover that setup with focused tests where practical.

## 2. Implement project and task repositories

- [x] 2.1 Implement the SQLite-backed `ProjectRepository` adapter with create, find, list, update, and soft-delete operations against the `projects` table.
- [x] 2.2 Implement the SQLite-backed `TaskRepository` adapter with create, find, list, update, and soft-delete operations against the `tasks` table.
- [x] 2.3 Add row-mapping helpers for `Project` and `Task`, including audit field reconstruction, optional `worktree_id` handling, and task status integer conversion.
- [x] 2.4 Add targeted `ora-db` tests that verify visible reads exclude soft-deleted project and task rows and that CRUD writes return the stored domain snapshots.

## 3. Implement session and worktree repositories

- [x] 3.1 Implement the SQLite-backed `SessionRepository` adapter with create, find, list, update, and soft-delete operations against the `sessions` table.
- [x] 3.2 Implement the SQLite-backed `WorktreeRepository` adapter with create, find, list, update, and soft-delete operations against the `worktrees` table.
- [x] 3.3 Add row-mapping helpers for `Session` and `Worktree`, including optional text fields plus session status and worktree activity integer conversions.
- [x] 3.4 Add targeted `ora-db` tests that verify visible reads exclude soft-deleted session and worktree rows and that CRUD writes return the stored domain snapshots.

## 4. Harden integration behavior and documentation

- [x] 4.1 Map SQLite execution and row-conversion failures into the matching application-owned repository error types for all four adapters.
- [x] 4.2 Add integration-style coverage that constructs multiple repositories from one `r2d2`-backed database pool and proves the exported adapter surface composes with the application ports.
- [x] 4.3 Update any affected documentation in `docs/` that still describes `ora-db` as the future home of these repository implementations.
- [x] 4.4 Run `cargo fmt --all` and `task test`, then fix any compile, formatting, or test regressions introduced by the new db adapters.
