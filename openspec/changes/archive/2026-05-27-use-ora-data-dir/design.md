# Design: Derive paths from `ORA_DATA_DIR` when present

Goals:
- Prefer a single `ORA_DATA_DIR` environment variable as the authoritative base for runtime state.
- If `ORA_DATA_DIR` is unset, treat it as the current working directory (`.`) instead of preserving the previous defaults.
- Use idiomatic Rust `Path`/`PathBuf` APIs (no string concatenation) when composing derived paths.

Changes to make in `apps/web/server/src/config.rs`:

1. Add a new environment variable constant:

   const DATA_DIR_ENV_VAR: &str = "ORA_DATA_DIR";

2. Update `DatabaseConfig::from_reader` logic to select the database path in this order:

   - If `ORA_DATA_DIR` exists, derive `$ORA_DATA_DIR/ora.sqlite3` via `PathBuf::from(data_dir).join("ora.sqlite3")`.
   - Else: treat `ORA_DATA_DIR` as the current working directory `.` and derive `./ora.sqlite3`.

   Validate non-empty path as before and construct a `PathBuf`.

3. Logging: update `read_logging_config` to compute the `file_config` path using:

   - If `ORA_DATA_DIR` is set, derive `$ORA_DATA_DIR/logs/ora.log` using `PathBuf::from(data_dir).join("logs").join("ora.log")`.
   - Else treat `ORA_DATA_DIR` as `.` and derive `./logs/ora.log`.

   Note: This intentionally changes the previous default log location from `./ora.log` to `./logs/ora.log` when `ORA_DATA_DIR` is not provided. Document this change in the proposal and release notes.

   Use the derived path as a `String` for `FileLoggingConfig::new()` (via `to_string_lossy()` if necessary).

4. Work dir: no change required to `ProjectConfig::from_reader`'s default behaviour because `default_work_dir()` derives `worktrees` next to the database file. When `ORA_DATA_DIR` drives the DB path (including when defaulting to `.`), the existing default yields `$ORA_DATA_DIR/worktrees` (e.g. `./worktrees`).

5. Tests:

   - Add tests that assert the derived DB path and log path when `ORA_DATA_DIR` is provided.
   - Add tests that confirm unset `ORA_DATA_DIR` falls back to `.` and yields `./ora.sqlite3`, `./worktrees`, and `./logs/ora.log`.

6. Implementation notes:

   - Use `read_variable(DATA_DIR_ENV_VAR)` as the only path root — do not preserve explicit `ORA_DB_PATH`, `ORA_WORK_DIR`, or `ORA_LOG_PATH` branches.
   - Compose paths with `PathBuf::join()`.
   - Avoid string interpolation for path separators; rely on `Path` APIs for cross-platform correctness.
