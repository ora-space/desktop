# Gitlancer Architecture

`gitlancer` is Ora's typed Git CLI runtime.

## Goals

- Make multi-worktree support a first-class capability instead of an afterthought.
- Provide stable typed request/response contracts for upper layers.
- Keep the implementation strictly on top of the Git CLI, without `libgit2`.
- Make execution observable, injectable, and easy to test.
- Prefer repository- and worktree-aware domain types that prevent invalid states.

## Design principles

1. **Model repository shapes explicitly.** Main worktrees, linked worktrees, repo roots, git dirs, and repo-relative paths are different concepts and use different types.
2. **Separate domain, execution, parsing, and Git use cases.** Command construction is not mixed with filesystem validation or stdout parsing.
3. **Keep requests and responses explicit.** Stable typed command boundaries serve Ora better than extension traits on a mutable handle.
4. **Prefer static dispatch.** `Git<R: GitRunner>` keeps the execution backend generic and testable without dynamic dispatch.
5. **Parse only stable Git outputs.** Porcelain and plumbing formats only.

## Layer responsibilities

### `domain`

Owns repository facts and invariants: `RepoRoot`, `WorktreeRoot`, `GitDir`, `RepoRelativePath`, `BranchName`, `CommitId`, `Repository`, `WorktreeHandle`, `WorktreeKind`.

It answers questions such as which repo a worktree belongs to, and whether a path is safe to pass to `git add` from this worktree. `WorktreeHandle::resolve_repo_relative_path` lexically normalizes a caller path and rejects absolute paths and traversal outside the worktree; it does not require the target to exist and does not canonicalize through the filesystem. `RepoRelativePath` is obtainable only through that worktree-aware boundary.

Constructors for repository and worktree roots assume their callers already validated Git identity —discovery and validation belong to `git`. This layer spawns no processes and parses no output.

### `exec`

Wraps Git CLI invocation: `GitCommand`, `GitIntent`, `GitEnv`, `GitOutput`, `GitRunner`, `CliGitRunner`, `RecordingGitRunner`.

It exists so upper layers can inject a fake runner in tests, record commands for debugging or telemetry, and distinguish read-only, mutating, and networked operations.

### `git`

Exposes the typed use cases Ora calls directly -repository discovery, worktree discovery and lifecycle, branch read and lifecycle, add/commit, diff, push, status, global identity, and repository sync (clone/fetch/checkout/fast-forward pull). Each takes a typed request and returns a typed response, which keeps option growth manageable and produces better call boundaries for agent orchestration.

Worktree-base discovery is a separate local branch read path because worktree selection must not have network or repository-state side effects.

### `parse`

Converts stable Git output into typed results, focusing on porcelain and plumbing formats and avoiding human-oriented messages. An empty or structurally incomplete required payload is a `ParseError`; parsers never invent missing identities. A detached worktree is represented by an absent branch rather than rejected.

## Core types

### Runtime

```rust
pub struct Git<R: GitRunner> {
    runner: R,
}
```

`Git` is the entry point for all Git use cases. It owns the execution strategy but no mutable repository state.

### Repository and worktree

```rust
pub struct Repository {
    root: RepoRoot,
}

pub struct WorktreeHandle {
    repo_root: RepoRoot,
    worktree_root: WorktreeRoot,
    git_dir: GitDir,
    kind: WorktreeKind,
    branch_name: Option<BranchName>,
}

pub enum WorktreeKind {
    Main,
    Linked { name: String },
}
```

This removes ambiguity between the repository root, the directory where a command should run, and the gitdir backing that worktree. Fields are private and read through accessors.

## Typed operations

### Worktree queries

`list_worktrees` parses `git worktree list --porcelain` and returns `WorktreeHandle` values whose `repo_root` points to the owning repository, whose `worktree_root` points to the checkout root, and whose `kind` distinguishes the main worktree from linked worktrees. A repository with its main checkout plus one linked worktree returns two handles, exactly one of them `WorktreeKind::Main`.

Three resolution paths exist:

- `resolve_worktree` finds a linked worktree by its configured worktree name.
- `resolve_worktree_by_branch` finds the worktree that has a given branch checked out. This is the path Ora's agent runtime uses to recover a task's checkout directory from its persisted branch name.
- `find_worktree` locates which worktree contains an arbitrary nested filesystem path.

### Inspection

- `list_branches` returns each local branch as a `BranchName`.
- `diff` returns a full-context unified patch for branch, unstaged, staged, or committed scopes. Untracked files are rendered without changing the caller's index, and output is bounded before it enters application memory.
- `list_worktree_bases` returns local `refs/heads` bases as `WorktreeBase` values. `resolve_worktree_base_commit` resolves the selected local ref directly to an immutable `CommitId`.
- `status` returns one `StatusEntry` per porcelain-v2 record, from `git status --porcelain=v2 -z`.
- `commit` returns the resulting `HEAD` commit id and the latest commit summary.
- `push_branch` publishes the checked-out branch to `origin` with `GitIntent::Network` and prompts disabled.
- `read_global_identity` returns `GlobalIdentity { name, email }`, treating an unset Git config key as `None` rather than an execution failure.

### Lifecycle

Branch and worktree lifecycle commands are typed APIs, so callers never assemble raw Git arguments:

- `create_branch` creates a branch at the caller-supplied `CommitId`; `delete_branch` uses `BranchDeletionMode` to select checked or forced deletion. A deleted branch no longer appears in `list_branches`.
- `create_worktree` creates its branch and linked checkout at the caller-supplied `CommitId`; `delete_worktree` uses `WorktreeDeletionMode` to select checked or forced removal. A created linked worktree appears in `list_worktrees` as `WorktreeKind::Linked`; a removed one disappears.
- `add` stages `RepoRelativePath` values; `commit` creates commits without GPG signing.
- `push_branch` is the only networked use case in this module. It derives the branch from `WorktreeHandle` rather than accepting a free-form ref from an upper layer.

Mutating operations perform domain validation _before_ invoking Git whenever the invalid state is determinable from repository and worktree metadata, and no Git command is issued when validation fails:

- Deleting the repository's main worktree returns `DomainError::CannotDeleteMainWorktree`.
- Deleting a linked worktree that belongs to a different repository returns `DomainError::WorktreeMismatch`.
- Creating a branch that already exists, or deleting one that does not, returns the corresponding `BranchAlreadyExists` / `BranchNotFound` domain error.

## Request / response style

Rather than attaching methods to a mutable worktree handle, gitlancer favors explicit request objects:

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

pub struct CreateBranchRequest<'a> {
    pub repository: &'a Repository,
    pub branch_name: BranchName,
    pub commit_id: CommitId,
}

pub struct CreateWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_root: WorktreeRoot,
    pub branch_name: BranchName,
    pub base_commit_id: CommitId,
}

pub struct DeleteWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree: &'a WorktreeHandle,
    pub mode: WorktreeDeletionMode,
}
```

Requests in this shape are easier to log, extend with options, serialize into agent tool payloads, and validate before execution. Every use case executes in a `Repository` or `WorktreeHandle` context, so a caller cannot confuse the main checkout, a linked checkout, and the shared repository root.

## Execution semantics

```rust
pub struct GitCommand {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: GitEnv,
    pub intent: GitIntent,
}
```

`GitIntent` is `ReadOnly`, `Mutating`, or `Network`. gitlancer classifies but does not enforce: Ora's upper layers use the intent to decide whether a command may run automatically, needs confirmation, or should be retried.

`GitEnv::automation_defaults` disables terminal prompts, fixes `LANG` to `C`, and disables paging, so an agent-driven command cannot block on interactive UI or return localized output.

`CliGitRunner` invokes the system `git` binary, measures duration, emits optional command telemetry through the logger registry, and returns a normalized `GitOutput { code, stdout, stderr, duration_ms }`. `run_bounded` drains stdout and stderr concurrently, kills Git after a stream exceeds its budget, and reports `GitExecError::OutputTooLarge`; the diff use case maps that to `GitlancerError::DiffTooLarge`. A non-zero exit retains the exit code, arguments, stdout, and stderr for upper-layer diagnostics. `RecordingGitRunner` executes nothing and exists for command-construction tests.

Command telemetry is opt-in through `gitlancer::logging::register`; `ora-logging` supplies the bridge Ora installs at startup. See [Runtime Logging](runtime-logging.md#git-command-logging).

## Parsing strategy

gitlancer relies on stable machine-readable outputs:

- `git worktree list --porcelain` -repository discovery, worktree listing, and worktree resolution
- `git for-each-ref refs/heads` -local worktree-base discovery without remote or repository-state side effects
- `git rev-parse <ref>^{commit}` -immutable worktree-base resolution
- `git status --porcelain=v2 -z` -status entries
- `git rev-parse HEAD` and `git log -1 --pretty=%s` -commit id and summary after a commit

`discover_repository` reads the main checkout out of the porcelain worktree list rather than calling `git rev-parse --show-toplevel`, so discovery from a nested directory and from inside a linked worktree both resolve to the owning repository root. A non-zero exit there is reported as `DomainError::NotARepository` rather than a raw execution error.

Human-readable stderr remains useful for diagnostics but is never the primary source of structured state.

## Error model

The public error hierarchy separates the three failure kinds:

```rust
GitlancerError
  - Domain(DomainError)
  - Exec(GitExecError)
  - Parse(ParseError)
```

Key variants:

- `DomainError::NotARepository`, `NotAWorktreeRoot`, `PathOutsideWorktree`, `WorktreeMismatch`, `CannotDeleteMainWorktree`, `BranchNotFound`, `BranchAlreadyExists`
- `GitExecError::GitNotFound`, `SpawnFailed`, `NonZeroExit`
- `GitExecError::OutputTooLarge`, `OutputReadFailed`, and `GitlancerError::DiffTooLarge`
- `ParseError::MissingLine`, `InvalidWorktreeList`, `InvalidStatus`

## Boundaries

gitlancer does not use `libgit2`, manage Ora database records, clean up database state after a Git failure, decide user-confirmation policy, or retry commands. Ora task and worktree lifecycle policy lives in `ora-application`; see [Task Worktrees](task-worktrees.md).

## Testing strategy

gitlancer is tested at three levels:

1. Unit tests for parsers and path/domain validation.
2. Fake-runner tests for command assembly and option handling.
3. Real Git integration tests for multi-worktree scenarios.

Priority integration scenarios: open a repository from a nested directory, list main and linked worktrees, discover and resolve local worktree bases, create branches and worktrees at explicit commits, add and commit from a linked worktree, detect worktree mismatch, parse `status --porcelain=v2 -z`, and handle linked-worktree `.git` indirection correctly.
