# Gitlancer Domain Module

This module models the repository and worktree facts required by typed Git operations without spawning processes or parsing command output.

## Core distinctions

- `RepoRoot` identifies the shared repository root.
- `WorktreeRoot` identifies the checkout where a worktree-scoped command executes.
- `GitDir` identifies the Git metadata associated with that checkout.
- `Repository` represents repository identity, while `WorktreeHandle` binds repository, checkout, Git metadata, kind, and optional checked-out branch.
- `WorktreeKind` distinguishes the main checkout from linked worktrees.
- `BranchName`, `CommitId`, and `RepoRelativePath` prevent unrelated string and path concepts from being mixed at call sites.

`WorktreeHandle::resolve_repo_relative_path` lexically normalizes caller paths through the shared `ora-utils` path helpers and rejects absolute or relative traversal outside the worktree. It does not require the target to exist and does not canonicalize through the filesystem. Callers obtain `RepoRelativePath` only through this worktree-aware boundary.

Constructors for repository and worktree roots assume their callers already validated Git identity. Discovery and validation belong to the `git` module; CLI invocation belongs to `exec`.

See the [gitlancer overview](../../README.md) and [Gitlancer Architecture](../../../../docs/gitlancer-architecture.md).
