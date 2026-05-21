## Why

The current runtime configuration scatters file paths across multiple environment variables (`ORA_DB_PATH`, `ORA_WORK_DIR`, `ORA_LOG_PATH`) and hardcoded defaults. This is confusing for deployment and local development. Introducing a single base data directory environment variable (`ORA_DATA_DIR`) simplifies configuration, reduces duplication, and standardizes where runtime data, worktrees, and logs are placed.

## What Changes

- Add a new environment variable `ORA_DATA_DIR` that, when set, becomes the canonical base for runtime files.
- Derive `ORA_DB_PATH`, `ORA_WORK_DIR`, and `ORA_LOG_PATH` from `ORA_DATA_DIR` as `$ORA_DATA_DIR/ora.sqlite3`, `$ORA_DATA_DIR/worktrees`, and `$ORA_DATA_DIR/logs/ora.log` respectively when those specific variables are not explicitly set.
- Preserve backward compatibility: if `ORA_DB_PATH`, `ORA_WORK_DIR`, or `ORA_LOG_PATH` are explicitly provided, they take precedence over values derived from `ORA_DATA_DIR`.
- Update configuration parsing in the web server runtime to support `ORA_DATA_DIR` and the new precedence rules.
- Update docs, tests, and examples to reference `ORA_DATA_DIR` usage and defaults.

## Capabilities

### New Capabilities
- `data-dir-config`: Standardize runtime filesystem layout around a single `ORA_DATA_DIR` environment variable. (This is an internal configuration convenience; it does not add public API surface.)

### Modified Capabilities
- `web-server-runtime`: Update requirements and documentation to describe `ORA_DATA_DIR` and the precedence rules for `ORA_DB_PATH`, `ORA_WORK_DIR`, and `ORA_LOG_PATH`.

## Impact

- Code changes: `apps/web/server/src/config.rs` (primary), `apps/web/server/src/main.rs` (tests/usage), and related unit tests.
- Documentation: `docs/web-server-runtime.md` and any README references.
- Tests: Add unit tests for `RuntimeConfig::from_reader` to validate `ORA_DATA_DIR` precedence and derived defaults.
- Backwards compatibility: Explicit environment variables continue to work; deploying with `ORA_DATA_DIR` is now recommended.
