# Proposal: Use `ORA_DATA_DIR` to derive runtime paths

What: Replace separate hard-coded environment-backed paths for the SQLite database, worktrees and log file with a single `ORA_DATA_DIR` base directory. When `ORA_DATA_DIR` is set, derive:

- `ORA_DB_PATH` -> `$ORA_DATA_DIR/ora.sqlite3`
- `ORA_WORK_DIR` -> `$ORA_DATA_DIR/worktrees`
- `ORA_LOG_PATH` -> `$ORA_DATA_DIR/logs/ora.log`

Why: The current code duplicates default path strings (`./ora.sqlite3`, `./ora.log`) and requires callers to set multiple environment variables to move runtime data. A single `ORA_DATA_DIR` makes deployments and testing simpler, avoids accidental hard-coded paths, and groups runtime state under one well-known directory. This reduces configuration surface and matches common application conventions.

Default when unset: If `ORA_DATA_DIR` is not set, treat it as the current working directory (`.`) and derive the same sibling paths from there (e.g. `./ora.sqlite3`, `./worktrees`, `./logs/ora.log`). This intentionally changes the previous implicit behavior for the log file (previously `./ora.log`) so logs are now stored under `./logs/ora.log` when `ORA_DATA_DIR` is not provided.

References:
- Issue: https://github.com/ora-space/desktop/issues/1
- File: [apps/web/server/src/config.rs](apps/web/server/src/config.rs#L1-L200)

Outcome: Create design and implementation tasks to update `config.rs` to prefer `ORA_DATA_DIR` when present and to default `ORA_DATA_DIR` to the current working directory when it is unset.
