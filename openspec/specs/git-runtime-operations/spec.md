## Purpose

Define the typed Git runtime operations that Gitlancer exposes for repository, branch, status, commit, and linked-worktree workflows.

## Requirements

### Requirement: Repository worktree queries return multi-worktree-aware handles
The runtime SHALL list repository worktrees from `git worktree list --porcelain` and return `WorktreeHandle` values whose `repo_root` points to the owning repository, whose `worktree_root` points to the checkout root, and whose `kind` distinguishes the main worktree from linked worktrees.

#### Scenario: Listing main and linked worktrees
- **WHEN** a repository contains its main checkout and one linked worktree
- **THEN** `list_worktrees` returns two worktrees
- **THEN** exactly one returned worktree is `WorktreeKind::Main`
- **THEN** the linked worktree is returned as `WorktreeKind::Linked`

### Requirement: Worktrees can be resolved by name and by nested path
The runtime SHALL resolve linked worktrees by their configured worktree name and SHALL locate which worktree contains an arbitrary nested filesystem path.

#### Scenario: Resolving a linked worktree by name
- **WHEN** a caller requests a linked worktree name that exists in the repository
- **THEN** `resolve_worktree` returns the corresponding `WorktreeHandle`

#### Scenario: Finding a worktree from a nested file path
- **WHEN** a caller provides a path nested under a linked worktree checkout
- **THEN** `find_worktree` returns the linked worktree that contains that path

### Requirement: Runtime inspection commands return typed branch, status, and commit data
The runtime SHALL expose local branches, status entries, and commit metadata through typed responses backed by machine-readable Git output.

#### Scenario: Listing local branches
- **WHEN** a repository contains multiple local branches
- **THEN** `list_branches` returns each local branch as a `BranchName`

#### Scenario: Reading structured status data
- **WHEN** a worktree contains tracked or untracked changes
- **THEN** `status` returns at least one `StatusEntry` for each porcelain-v2 status record

#### Scenario: Reading commit metadata after a successful commit
- **WHEN** `commit` succeeds in a worktree
- **THEN** the response includes the `HEAD` commit ID
- **THEN** the response includes the latest commit summary

### Requirement: Runtime branch lifecycle commands are exposed through typed repository APIs
The runtime SHALL expose typed APIs to create and delete local branches from repository-aware inputs without requiring callers to assemble raw Git arguments.

#### Scenario: Creating a local branch
- **WHEN** a caller requests creation of a new local branch in a repository
- **THEN** the runtime creates that branch through the Git CLI
- **THEN** the response identifies the created branch as a `BranchName`

#### Scenario: Deleting a local branch
- **WHEN** a caller requests deletion of an existing local branch in a repository
- **THEN** the runtime deletes that branch through the Git CLI
- **THEN** the deleted branch no longer appears in `list_branches`

### Requirement: Runtime worktree lifecycle commands manage linked worktrees explicitly
The runtime SHALL expose typed APIs to create and delete linked worktrees while preserving the distinction between the main worktree and linked worktrees.

#### Scenario: Creating a linked worktree
- **WHEN** a caller requests creation of a linked worktree for a repository at a target checkout path
- **THEN** the runtime creates the linked worktree through the Git CLI
- **THEN** `list_worktrees` returns the new worktree as `WorktreeKind::Linked`

#### Scenario: Deleting a linked worktree
- **WHEN** a caller requests deletion of an existing linked worktree that belongs to a repository
- **THEN** the runtime removes that linked worktree through the Git CLI
- **THEN** `list_worktrees` no longer returns the removed worktree

### Requirement: Lifecycle mutations reject invalid destructive targets through typed errors
The runtime SHALL reject unsupported or mismatched lifecycle requests with typed validation errors before invoking Git whenever the invalid state can be determined from repository and worktree metadata.

#### Scenario: Rejecting deletion of the main worktree
- **WHEN** a caller requests deletion of the repository's main worktree
- **THEN** the runtime returns a domain validation error
- **THEN** no Git deletion command is invoked

#### Scenario: Rejecting removal of a worktree from another repository
- **WHEN** a caller requests deletion of a linked worktree that does not belong to the supplied repository
- **THEN** the runtime returns a domain validation error
- **THEN** no Git deletion command is invoked