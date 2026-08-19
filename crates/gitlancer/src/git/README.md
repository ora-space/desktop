# Typed Git Use Cases

This module exposes the operations callers perform through `Git<R: GitRunner>` and keeps command construction behind typed request and response objects.

## Responsibilities

- Repository operations discover a Git root, open an already validated repository, and list its worktrees.
- Worktree operations resolve by path or branch and create or delete linked worktrees with explicit deletion modes.
- Branch operations list, validate, create, and delete local branches with checked or forced semantics.
- Worktree-base operations list local refs and resolve a selected base to an immutable commit without network or repository-state side effects.
- Diff operations produce standard unified patches for branch, unstaged, staged, and committed scopes, including untracked files without modifying the caller's index.
- Commit operations stage `RepoRelativePath` values, create commits without GPG signing, and return typed commit metadata.
- Push operations publish the verified checked-out branch to its default remote without enabling credential prompts.
- Status uses porcelain v2 NUL-delimited output; global identity reads treat an unset Git key as `None`, not an execution failure.
- Sync operations clone a repository into a fresh directory, fetch a remote, check out a branch, and fast-forward a branch against its remote so marketplace sources can be refreshed deterministically.

Every command is classified by `GitIntent` and executed through the injected runner. Mutating operations perform domain checks such as duplicate/missing branch validation before issuing the mutation when the use case requires it.

Use cases execute in a `Repository` or `WorktreeHandle` context so callers cannot accidentally confuse the main checkout, a linked checkout, or the shared repository root. Output conversion is delegated to the `parse` module.

Diff output is bounded before it reaches application memory, and an oversized patch is reported as `GitlancerError::DiffTooLarge`. This module does not persist Ora tasks or worktrees, choose user-confirmation policy, retry commands, or clean up database state after Git failure.

See the [gitlancer overview](../../README.md) and [Gitlancer Architecture](../../../../docs/gitlancer-architecture.md).
