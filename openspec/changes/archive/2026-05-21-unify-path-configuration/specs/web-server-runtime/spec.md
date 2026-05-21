## ADDED Requirements

### Requirement: Web server runtime SHALL use a unified data directory when configured

The system SHALL accept an optional `ORA_DATA_DIR` environment variable acting as the canonical runtime data root. When `ORA_DATA_DIR` is set, the runtime SHALL derive other filesystem configuration values from it unless explicit overrides are provided.

Resolution / precedence:
- If `ORA_DB_PATH` is explicitly set, the runtime SHALL use it.
- Otherwise, if `ORA_DATA_DIR` is set, the runtime SHALL use `$ORA_DATA_DIR/ora.sqlite3` as the database path.
- Otherwise, the runtime SHALL fall back to the existing default `./ora.sqlite3`.

Similarly for linked worktrees and logs:
- `ORA_WORK_DIR` explicit value takes precedence.
- Otherwise, if `ORA_DATA_DIR` is set, the runtime SHALL use `$ORA_DATA_DIR/worktrees`.
- Otherwise, the runtime SHALL default to a `worktrees` sibling of the configured database path.

- `ORA_LOG_PATH` explicit value takes precedence.
- Otherwise, if `ORA_DATA_DIR` is set, the runtime SHALL use `$ORA_DATA_DIR/logs/ora.log`.
- Otherwise, the runtime SHALL use the existing default `./ora.log`.

#### Scenario: `ORA_DATA_DIR` is set and no explicit overrides
- WHEN the process starts with `ORA_DATA_DIR=/var/lib/ora`
- THEN the database path SHALL be `/var/lib/ora/ora.sqlite3`, the worktree root SHALL be `/var/lib/ora/worktrees`, and the log path SHALL be `/var/lib/ora/logs/ora.log` (created or ensured writable by the runtime during bootstrap where required).

#### Scenario: explicit override takes precedence
- WHEN `ORA_DATA_DIR=/var/lib/ora` and `ORA_DB_PATH=/data/ora.sqlite3` are both set
- THEN the runtime SHALL use `/data/ora.sqlite3` for its database and still derive worktrees/log paths from `ORA_DATA_DIR` unless their explicit globals are set.
