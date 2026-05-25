# Tasks: Implement `ORA_DATA_DIR`-derived paths

- [x] Add `ORA_DATA_DIR` as the single path root in `apps/web/server/src/config.rs`.
- [x] Derive the database path, worktree path, and log path from `ORA_DATA_DIR`, with `.` as the fallback when unset.
- [x] Add or update tests for the derived paths and the unset fallback.
- [x] Run formatting and the relevant test command.