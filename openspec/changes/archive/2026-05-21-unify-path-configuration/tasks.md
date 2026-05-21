## Implementation Tasks

1. Update configuration parsing
   - Modify `apps/web/server/src/config.rs` so `DatabaseConfig::from_reader`, `ProjectConfig::from_reader`, and `read_logging_config` consult `ORA_DATA_DIR` as described in the design and spec.
   - Preserve precedence: explicit `ORA_DB_PATH`, `ORA_WORK_DIR`, `ORA_LOG_PATH` override derived values.
   - Ensure path building is platform-agnostic (use `PathBuf::join`).

2. Ensure logging and worktree directories are created when necessary
   - During bootstrap, ensure parent directories for the configured log file and `work_dir` exist or are created with reasonable permissions.

3. Add unit tests
   - Add tests in `apps/web/server/src/config.rs`'s test module to validate:
     - Default behavior when no env vars provided
     - Behavior when only `ORA_DATA_DIR` provided
     - Precedence behavior when explicit vars are provided alongside `ORA_DATA_DIR`

4. Update documentation
   - Update `docs/web-server-runtime.md` to document `ORA_DATA_DIR`, derived paths, and precedence rules with examples.

5. Run formatting and tests
   - Run `cargo fmt --all` and `task test` to ensure no regressions.

6. Release notes / changelog
   - Add a brief note to the repo changelog or release notes indicating the new recommended `ORA_DATA_DIR` and the precedence rules.

## Acceptance criteria

- `RuntimeConfig::from_reader` exposes correct path values according to the precedence rules.
- Unit tests cover the major precedence scenarios.
- Documentation clearly demonstrates `ORA_DATA_DIR` usage and examples.
- `task test` completes successfully.
