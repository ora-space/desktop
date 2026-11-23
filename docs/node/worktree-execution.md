# Worktree Execution

English | [中文](worktree-execution.zh.md)

Besides cloning, a Node can create and remove task worktrees under an existing Main Workspace it was
explicitly bound to (`ensure_worktree`, `remove_worktree`, capability `worktree_execution`). This was
the Node's first execution family; its
[decision](https://github.com/ora-space/specs/blob/main/decisions/node/worktree/0-worktree-management.md)
is superseded and the Cloud path uses [clones](repository-clone.md), but the family remains
implemented and shares the same durability rules. Desktop's own task worktrees are a different,
in-process path, see [Tasks and worktrees](../workspaces/tasks-and-worktrees.md).

## Bindings and targets

`node.repositories` lists `RepositoryBinding`s: a repository reference, the existing Main Workspace
identity and path, an authorized root and a managed worktree root. A command must match a binding;
the Node verifies the main checkout with Git and requires Git's main registration to match the
binding (merely running Git successfully inside a directory is not enough; linked metadata is checked
through its `commondir` and `gitdir` back-reference).

`NodeManaged.directory_name` must be one portable directory segment. Paths are built with
`Path`/`PathBuf` and shared canonical containment; static symlinks and overlap with the main checkout
are rejected. Static checks do not protect against a malicious symlink swap between validation and use.

Acceptance freezes the target: binding paths, the Git metadata directory, authorized and managed
roots, the task path, the branch and the immutable base commit. Retransmission never re-resolves a
moved base ref or replaces a saved path, and every mutation and recovery rechecks these facts.

## Execution

Input is durable before Git changes anything, and Git never runs inside a SQLite transaction. The
order is: validate and deduplicate, resolve, durably accept and reserve resources (Workspace, path,
branch), persist mutation intent, run Git as a recorded attempt (see
[Node](node.md#execution-and-recovery)), inspect the actual facts, then complete atomically with the
original event.

- Creation runs at the frozen path, branch and commit and verifies the resulting checkout; ambiguous
  partial effects are left for recovery.
- Removal force-removes only owned linked checkouts and task branches in independently recoverable
  stages, and reports success only after confirming the registration, directory and local branch are
  gone (removing only owned empty residual directories). `AlreadyAbsent` requires every target to be
  absent. Main worktrees, other branches and unowned non-empty directories are never force-cleaned.
- Precondition failures commit a structured failure and event; malformed messages and identity
  conflicts are rejected at entry. A definitive no-effect creation failure retires its reservations
  while keeping the input and result.

## Recovery

| Evidence | Status | Next action |
|---|---|---|
| Input saved, Git not started | `Accepted` | Persist running intent before mutating |
| Mutation may have happened | `Running` | Continue, or reconcile after restart |
| Insufficient evidence | `Unknown` | Observe again; complete only with sufficient facts |
| Result and event committed | `Completed` | Return the original result and replay the event |

Recovery keeps the original identities and targets and never picks a new path, branch or base:

- A valid owned checkout matching registration, path and branch completes creation, even with new
  commits, keeping the original base.
- A removed checkout with its owned branch remaining continues branch cleanup.
- An owned branch-only residue at the frozen base, with no conflicting checkout or unexplained files,
  is cleaned under persisted `CleanupCreation` intent and retried once per pass with the original
  execution.
- Inconsistent registration, changed bindings or failed inspection stay `Unknown` with diagnostics,
  never a terminal `ResultUnknown`. Resources reappearing under retired ownership make a new cleanup
  operation fail with `WorktreeConflict` without touching them.
- `Unknown` blocks only conflicting resources; unrelated work stays admissible.

A changed operation type or input under one execution, or an operation rebound to another execution,
returns `IdentityConflict` and preserves the old data. `RequestId` is correlation only.

## Interface

`Node::open(NodeConfig, ProcessConfig, Shutdown)` composes managed Git and Ora's local clock; tests use
`Node::open_with_dependencies` with injected Git, write-guard and clock. `submit(Command::Ensure | Remove)`
returns an `ExecutionStatus`, `status` reads durable evidence, `pending_events` and `acknowledge` handle
delivery, and `state` reports `Ready` or `RecoveryPending` while `recover` reconciles incomplete
records. Mutations take `&mut Node`, so callers share it behind a mutex.
