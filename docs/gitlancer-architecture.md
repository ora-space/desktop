# Gitlancer Architecture

## Goals

`gitlancer` is designed as a Git CLI runtime for Ora, an AI-agent-oriented IDE.
The main goals are:

- Make multi-worktree support a first-class capability instead of an afterthought.
- Provide stable typed request/response contracts for upper layers.
- Keep the implementation strictly on top of the Git CLI, without `libgit2`.
- Make execution observable, injectable, and easy to test.
- Prefer repository- and worktree-aware domain types that prevent invalid states.

## Design Principles

1. Model repository shapes explicitly.
   Main worktrees, linked worktrees, repo roots, git dirs, and repo-relative paths are different concepts and should use different types.
2. Separate domain, execution, parsing, and Git use cases.
   Command construction should not be mixed with filesystem validation or stdout parsing.
3. Keep requests and responses explicit.
   Ora will benefit from stable typed command boundaries more than extension traits on a mutable handle.
4. Prefer static dispatch.
   `Git<R: GitRunner>` keeps the execution backend generic and testable without dynamic dispatch.
5. Parse only stable Git outputs.
   Prefer porcelain and plumbing commands such as `git worktree list --porcelain`, `git status --porcelain=v2 -z`, and `git rev-parse`.

## Layer Responsibilities

### `domain`

The domain layer owns repository facts and invariants:

- `RepoRoot`, `WorktreeRoot`, `GitDir`
- `RepoRelativePath`
- `Repository`
- `WorktreeHandle`
- `WorktreeKind`

This layer should answer questions such as:

- Is this path a repository root?
- Which repo does this worktree belong to?
- Is this path safe to pass to `git add` from this worktree?

It should not spawn processes or parse command output directly.

### `exec`

The execution layer wraps Git CLI invocation:

- `GitCommand`
- `GitIntent`
- `GitEnv`
- `GitOutput`
- `GitRunner`
- `CliGitRunner`

This layer exists so upper layers can:

- inject a fake runner in tests,
- record commands for debugging or telemetry,
- distinguish read-only, mutating, and networked Git operations.

### `git`

The Git layer exposes typed use cases that Ora can call directly:

- repository discovery,
- worktree discovery and selection,
- add / commit / status,
- branch-oriented read and lifecycle flows,
- linked-worktree lifecycle flows.

Each use case should take a typed request object and return a typed response object.
That keeps option growth manageable and produces better call boundaries for agent orchestration.

### `parse`

The parse layer converts stable Git output into typed results.
It should focus on porcelain/plumbing formats and avoid parsing human-oriented messages whenever possible.

## Core Types

### Runtime

```rust
pub struct Git<R: GitRunner> {
    runner: R,
}
```

`Git` is the entry point for all Git use cases.
It owns the execution strategy but not repository state.

### Repository and Worktree

```rust
pub struct Repository {
    root: RepoRoot,
}

pub struct WorktreeHandle {
    repo_root: RepoRoot,
    worktree_root: WorktreeRoot,
    git_dir: GitDir,
    kind: WorktreeKind,
}

pub enum WorktreeKind {
    Main,
    Linked { name: String },
}
```

This structure makes multi-worktree support explicit and removes ambiguity between:

- the repository root,
- the directory where a command should run,
- the gitdir backing that worktree.

## Request / Response Style

Instead of attaching methods directly to `Worktree`, gitlancer favors explicit request objects:

```rust
pub struct AddRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub paths: Vec<RepoRelativePath>,
}

pub struct CommitRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub message: &'a str,
    pub allow_empty: bool,
}

pub struct ListWorktreesRequest<'a> {
    pub repository: &'a Repository,
}

pub struct CreateBranchRequest<'a> {
    pub repository: &'a Repository,
    pub branch_name: BranchName,
}

pub struct DeleteWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree: &'a WorktreeHandle,
    pub mode: WorktreeDeletionMode,
}
```

This is a better fit for Ora because requests are:

- easier to log,
- easier to extend with options,
- easier to serialize into agent tool payloads,
- easier to validate before execution.

## Execution Semantics

`GitCommand` should carry enough metadata for policy and observability:

```rust
pub struct GitCommand {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: GitEnv,
    pub intent: GitIntent,
}
```

Suggested intents:

- `ReadOnly`
- `Mutating`
- `Network`

Ora can use those intents to decide whether a command can run automatically, needs confirmation, or should be retried.

## Parsing Strategy

gitlancer should rely on stable machine-readable outputs:

- `git rev-parse --show-toplevel`
- `git rev-parse --git-dir`
- `git worktree list --porcelain`
- `git status --porcelain=v2 -z`
- `git rev-parse HEAD`
- `git log -1 --pretty=%s`

Human-readable stderr remains useful for diagnostics, but it should not be the primary source of structured state.

## Error Model

The public error hierarchy should clearly distinguish:

- domain validation failures,
- process spawning or execution failures,
- parsing failures.

Suggested shape:

```rust
GitlancerError
  - Domain(DomainError)
  - Exec(GitExecError)
  - Parse(ParseError)
```

Key examples:

- `DomainError::NotARepository`
- `DomainError::PathOutsideWorktree`
- `DomainError::WorktreeMismatch`
- `GitExecError::GitNotFound`
- `GitExecError::SpawnFailed`
- `GitExecError::NonZeroExit`
- `ParseError::InvalidWorktreeList`

## Testing Strategy

gitlancer should be tested at three levels:

1. Unit tests for parsers and path/domain validation.
2. Fake-runner tests for command assembly and option handling.
3. Real Git integration tests for multi-worktree scenarios.

Priority integration scenarios:

- open repository from nested directory,
- list main and linked worktrees,
- add and commit from a linked worktree,
- detect worktree mismatch,
- parse `status --porcelain=v2 -z`,
- handle linked-worktree `.git` indirection correctly.

## Notes

The Rust skeleton added alongside this document is intentionally light on behavior and heavy on boundaries, so the next implementation step can focus on filling in domain validation, command assembly, and parsing logic without redesigning the module graph again.