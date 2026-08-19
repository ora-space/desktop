# ora-test-support

`ora-test-support` provides test-only fixtures for integration tests that exercise real Git repositories and linked worktrees.

The crate owns isolated temporary repository roots, deterministic Git configuration, fixture file writes, commits, and linked-worktree creation. Each `GitTestScaffold` keeps its global Git configuration inside its temporary sandbox and removes the sandbox through its `TempDir` owner.

Production crates must depend on this crate only through `dev-dependencies`. It does not contain Ora domain behavior, production repository adapters, or runtime Git orchestration.
