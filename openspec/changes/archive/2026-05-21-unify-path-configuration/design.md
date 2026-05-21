## Context

The web server runtime currently reads several independent environment variables for filesystem locations (`ORA_DB_PATH`, `ORA_WORK_DIR`, `ORA_LOG_PATH`) and falls back to hardcoded defaults. This disperses configuration and makes deployments and local runs inconsistent. The change introduces `ORA_DATA_DIR` as a single authoritative data root, while preserving explicit overrides.

Files of interest:
- `apps/web/server/src/config.rs` — current configuration parsing and defaults.
- `docs/web-server-runtime.md` — documentation to update.

## Goals / Non-Goals

**Goals:**
- Provide a single `ORA_DATA_DIR` environment variable that can determine the default locations for DB, worktrees, and logs.
- Keep explicit variables (`ORA_DB_PATH`, `ORA_WORK_DIR`, `ORA_LOG_PATH`) as overrides with higher precedence.
- Add unit tests for the precedence and derived defaults.

**Non-Goals:**
- Change storage formats or migration behavior of the SQLite DB.
- Change runtime behavior beyond path derivation (no change to HTTP API or application semantics).

## Decisions

1. Precedence order

- `Explicit env var (ORA_DB_PATH / ORA_WORK_DIR / ORA_LOG_PATH)` — highest precedence
- `Derived from ORA_DATA_DIR` — used when explicit vars are absent
- `Existing defaults` (`./ora.sqlite3`, `./worktrees`, `./ora.log`) — fallback

Rationale: preserves backwards compatibility while making a single opt-in way to relocate all runtime state.

2. Placement and naming

- Derive DB path as `$ORA_DATA_DIR/ora.sqlite3`.
- Derive worktrees as `$ORA_DATA_DIR/worktrees`.
- Derive logs as `$ORA_DATA_DIR/logs/ora.log`.

3. Implementation surface

- Update `DatabaseConfig::from_reader` to consult `ORA_DATA_DIR` when `ORA_DB_PATH` is not set.
- Update `ProjectConfig::from_reader` to derive `work_dir` from `ORA_DATA_DIR` (via `default_work_dir`) when appropriate.
- Update `read_logging_config` to use `ORA_DATA_DIR/logs/ora.log` when `ORA_LOG_PATH` is unset.

4. Path normalization and safety

- Normalize paths using `PathBuf::canonicalize` where appropriate in tests, but avoid forcing canonicalization at runtime to preserve behavior with non-existent paths in containers.

## Risks / Trade-offs

- Risk: Users may have relied on hardcoded defaults; recommend documenting the new preferred `ORA_DATA_DIR` and the precedence to avoid surprises.
- Risk: Deriving a log file path inside a directory that lacks permissions may cause logging initialization to fail; mitigation: document permissions and ensure logging code creates parent directories.

## Migration Plan

1. Code: implement the config changes and tests.
2. Docs: update `docs/web-server-runtime.md` with `ORA_DATA_DIR` usage and examples.
3. Release: communicate the preferred `ORA_DATA_DIR` in changelog and onboarding docs.

## Open Questions

- Should `ORA_DATA_DIR` accept relative paths (yes, but document as recommended absolute)?
- Should we create directories automatically (worktrees/logs) during bootstrap? (Design suggests creating worktrees root; logging code may need to create parent directory).
