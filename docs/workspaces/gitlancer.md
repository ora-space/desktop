# Gitlancer

English | [中文](gitlancer.zh.md)

`gitlancer` (`crates/gitlancer`) is Ora's typed Git runtime. It runs on top of the Git CLI (no
`libgit2`), treats multiple worktrees as a first-class shape, and gives upper layers typed requests
and responses instead of raw arguments. It does not manage Ora records, clean up database state
after a Git failure, decide confirmation policy or retry commands; task and worktree policy lives in
`ora-application` (see [Tasks and worktrees](tasks-and-worktrees.md)).

## Layers

| Module   | Owns                                                                                                                                                                |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `domain` | Repository facts and invariants: `RepoRoot`, `WorktreeRoot`, `GitDir`, `RepoRelativePath`, `BranchName`, `CommitId`, `Repository`, `WorktreeHandle`, `WorktreeKind` |
| `exec`   | CLI invocation: `GitCommand`, `GitIntent`, `GitEnv`, `GitOutput`, `GitRunner`, `CliGitRunner`, `RecordingGitRunner`                                                 |
| `git`    | The typed use cases Ora calls                                                                                                                                       |
| `parse`  | Converting stable porcelain and plumbing output into typed results                                                                                                  |

`domain` spawns no process and parses no output. `WorktreeHandle::resolve_repo_relative_path`
lexically normalizes a path and rejects absolute paths and traversal outside the worktree, so a
`RepoRelativePath` can only be obtained through that worktree-aware boundary. Root constructors
assume the caller already validated Git identity; discovery belongs to `git`.

`Git<R: GitRunner>` is the entry point for every use case: it owns the execution strategy but no
mutable repository state, and static dispatch keeps the runner injectable for tests.

```rust
pub struct WorktreeHandle {
    repo_root: RepoRoot,
    worktree_root: WorktreeRoot,
    git_dir: GitDir,
    kind: WorktreeKind, // Main | Linked { name }
    branch_name: Option<BranchName>,
}
```

This separates the repository root, the directory a command runs in and the gitdir backing that
worktree. Every use case executes in a `Repository` or `WorktreeHandle` context.

## Use cases

- **Repositories and worktrees**: `discover_repository` reads the main checkout from `git worktree
list --porcelain` (not `rev-parse --show-toplevel`), so discovery from a nested directory or from
  inside a linked worktree resolves the owning repository. `list_worktrees` returns one handle per
  checkout with exactly one `WorktreeKind::Main`. `resolve_worktree` finds a linked worktree by
  name, `resolve_worktree_by_branch` by its checked-out branch (how Ora recovers a task checkout),
  and `find_worktree` the worktree containing a nested path.
- **Branches and bases**: `list_branches` returns local branches. `list_worktree_bases` reads local
  `refs/heads` and `resolve_worktree_base_commit` resolves a selected ref to a `CommitId`, with no
  network or repository-state side effects.
- **Lifecycle**: `create_branch` and `create_worktree` work at a caller-supplied `CommitId`;
  `delete_branch` and `delete_worktree` take an explicit checked or forced deletion mode.
- **Review**: `diff` returns a full-context unified patch for branch, unstaged, staged or committed
  scopes, rendering untracked files without touching the caller's index; `status` returns one entry
  per `git status --porcelain=v2 -z` record; `add` stages `RepoRelativePath` values; `commit`
  creates unsigned commits and returns the new `HEAD` and summary.
- **Network**: `push_branch` pushes the branch derived from the `WorktreeHandle` to `origin` with
  prompts disabled. Sync operations clone, fetch, check out and fast-forward, which marketplace
  sources use.
- **Checkpoints**: snapshot a worktree into `refs/ora/checkpoints/<name>` through a temporary index,
  list paths changed since that commit, and restore selected or all paths without touching the
  caller's index. Workflow runs use them, see [Workflow runs](../workflows/workflow-runs.md#rollback).
- **Identity**: `read_global_identity` returns name and email, treating an unset key as `None`.

Mutations validate before invoking Git whenever the invalid state is visible in repository
metadata, and issue no command when validation fails: deleting the main worktree
(`MainWorktreeDeletionUnsupported`), deleting a worktree of another repository
(`WorktreeMismatch`), creating an existing branch or deleting a missing one
(`BranchAlreadyExists`, `BranchNotFound`).

Requests are explicit objects rather than methods on a mutable handle, which makes them easy to log,
extend and validate:

```rust
pub struct CreateWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_root: WorktreeRoot,
    pub branch_name: BranchName,
    pub base_commit_id: CommitId,
}
```

## Execution

```rust
pub struct GitCommand {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: GitEnv,
    pub intent: GitIntent, // ReadOnly | Mutating | Network
}
```

`GitIntent` classifies but does not enforce; upper layers use it to decide whether a command may run
automatically, needs confirmation or may be retried. `GitEnv::automation_defaults` disables terminal
prompts, fixes `LANG=C` and `LC_ALL=C` (so a localized caller environment cannot override it) and
disables paging, so an agent-driven command cannot block on interactive UI or return localized
output.

`CliGitRunner` runs the system `git`, measures duration, reports optional telemetry and returns
`GitOutput { code, stdout, stderr, duration_ms }`. `run_bounded` drains stdout and stderr
concurrently and kills Git when a stream exceeds its budget (`GitExecError::OutputTooLarge`, mapped
to `GitlancerError::DiffTooLarge` for diffs). A non-zero exit keeps the code, arguments and output
for diagnostics. `RecordingGitRunner` executes nothing and exists for command-construction tests.
Telemetry is opt-in through `gitlancer::logging::register`, see
[Logging](../core/logging.md#git-command-logging).

## Parsing

Only stable machine-readable output is parsed: `git worktree list --porcelain`, `git for-each-ref
refs/heads`, `git rev-parse <ref>^{commit}`, `git status --porcelain=v2 -z`, and `git rev-parse
HEAD` plus `git log -1 --pretty=%s` after a commit. An empty or structurally incomplete payload is a
`ParseError`; parsers never invent identities. A detached worktree has an absent branch.
Human-readable stderr is kept for diagnostics but never used as structured state.

## Errors

```text
GitlancerError
  - Domain(DomainError)   NotARepository, NotAWorktree, PathOutsideWorktree, WorktreeMismatch,
                          MainWorktreeDeletionUnsupported, BranchNotFound, BranchAlreadyExists
  - Exec(GitExecError)    GitNotFound, SpawnFailed, NonZeroExit, OutputTooLarge, OutputReadFailed
  - Parse(ParseError)     MissingLine, InvalidWorktreeList, InvalidStatus, InvalidCheckpoint
  - DiffTooLarge, CommitMetadataUnavailable, Io
```

## Testing

Parsers and domain validation have unit tests, command assembly is tested with
`RecordingGitRunner`, and real-repository integration tests cover multi-worktree scenarios: nested
discovery, main and linked listing, base resolution, creation at explicit commits, committing from a
linked worktree, mismatch detection, porcelain v2 status and linked-worktree `.git` indirection.
