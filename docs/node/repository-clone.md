# Repository Clone

English | [中文](repository-clone.zh.md)

A Node clones a Workspace's code from a snapshot on the platform Git Server into a directory it
allocates, and reports the snapshot's base commit as the checkout's commit. Clone is its own execution
family with its own records; it needs no existing Main Workspace. The decisions are the
[clone root decision](https://github.com/ora-space/specs/blob/main/decisions/node/repository/0-clone-selected-repository-branch.md),
the [minimal clone execution contract](https://github.com/ora-space/specs/blob/main/decisions/node/repository/20260918-minimal-clone-execution-contract.md),
[a terminated attempt fails as interrupted](https://github.com/ora-space/specs/blob/main/decisions/node/repository/20260925-terminated-clone-attempt-fails-as-interrupted.md)
and [clone from a Git Server snapshot and ora-git](https://github.com/ora-space/specs/blob/main/decisions/node/repository/20261016-clone-from-git-server-snapshot-and-ora-git.md)
(`approved`; it becomes `implemented` after end-to-end acceptance). Agents reach the Git Server during a session through [ora-git](ora-git.md).

## Input

Cloud freezes the clone input from a Binding snapshot before it hands out a Workspace's clone step,
and the Controller forwards it verbatim (see [Controller](../controller/controller.md#workspace-sandboxes)).
`clone_repository` carries `node_id`, `repository_url`, `snapshot_ref`, `base_commit` and
`workspace_ref`:

- `repository_url` is an `https://` URL without userinfo, query or fragment;
- `snapshot_ref` names a canonical Binding UUID and exactly `base_commit`;
- `workspace_ref` is `refs/heads/workspaces/<workspaceId>/<branch>`, a canonical Workspace UUID and a
  concrete branch (not `HEAD`).

The former `repository`/`branch` input and SSH sources are rejected. See
[Controller–Node protocol](protocol.md#message-contracts).

## Deployment

```json
{
  "clone": {
    "repository_root": "/var/lib/ora/repositories",
    "git_config": "/etc/ora/clone.gitconfig",
    "search_path": ["/usr/bin"]
  },
  "git_access": {
    "ca_cert": "/run/ora/git-ca.pem",
    "client_cert": "/run/ora/git-client.pem",
    "client_key": "/run/ora/git-client-key.pem"
  }
}
```

- The Node advertises `repository_clone` only when both `clone` and `git_access` are configured; a
  Cloud-mode Node (`control.target` set) without `git_access` refuses to start with
  `git_access_required`. The git-access material is described in
  [Revisions](revisions.md#git-server-access-material).
- Provision the root and configuration before startup; the Node never creates or re-permissions them.
  Every path is absolute, owned by the current user or root and free of symlink components; release
  builds reject group- or other-writable paths. The root cannot overlap Node state, a configured
  legacy `process.host_directory` or configured worktree checkouts.
- The filesystem must report birth time: device, inode and birth time identify the root and each newly
  created leaf. Missing or replaced ownership evidence stays `Unknown`; a database reservation alone
  never owns a directory.
- `git_config` may select only non-secret CA trust, see [Runtime control](../controller/runtime-control.md).
  There is no SSH setting: clones come only from the Git Server over HTTPS, and SSH is disabled for
  every clone command.
- `process.command_timeout_ms` bounds local Git commands; network work uses `process.network_timeout_ms`
  (default 600000, allowed 1..1200000). An expired budget kills the command's process group, and the
  attempt still needs its stop evidence before the clone reports interrupted.

Embedding callers use `configure_clone`, `submit_clone` and `recover_clones`; the executable recovers
accepted clones at startup and takes new ones only from the control session.

## Execution

Each accepted request owns a new, exclusively created directory. A clone has two halves:

1. **Snapshot fetch, as the Node.** The Node fetches only `snapshot_ref` into a private bare transport
   repository under `<repository_root>/.ora-transport/`. The git-access certificate, key and CA and the
   clone execution's `Ora-Execution-Id` header are passed only as arguments of that command; credential
   helpers, redirects, hooks and ambient configuration are disabled. The fetched commit must equal
   `base_commit`. The transport repository holds no credential and is then handed to the workload user.
2. **Checkout, as the workload user.** Three recorded workload commands initialize the checkout,
   fetch the transport repository's objects into the local branch named after `workspace_ref`'s branch
   (the file transport is allowed only for that local fetch) and reset the working tree. Git runs with
   no templates, no recursive submodules and no LFS smudge or process filters; hooks and interactive
   prompts are disabled per command.

The checkout has full history and independent objects, no remote, no credential configuration and no
tracking refs. Verification checks its directory identity, a normal non-shallow repository, the branch,
clean tracked files and that HEAD equals `base_commit`.

Transport repositories are removed after use and cleared at startup, except the one of a clone whose
workload sequence has not finished the step that reads it: that step is never preceded by another
snapshot fetch, and its Git command may still be reading the transport.

## Failures and recovery

The snapshot fetch only changes the private transport repository, so it may simply start again after a
restart, once no unrecorded command of the previous Node process still holds its attempt lock.
Retries within a run and fetches after a restart share one durable attempt count: each attempt
adds one to it before it starts, and after three failed attempts in all the clone ends as
`source_unavailable`. Unreachable-server and 5xx failures are retried after 5 and then 30 seconds;
after a restart the next attempt starts at once instead of waiting out the pause. 401, 403 and a
missing ref are not retried.

| Code | Meaning |
|---|---|
| `snapshot_unavailable` | The Git Server says the snapshot ref is missing or not readable (it gives both the same answer), or refuses the Node's git-access identity (401) |
| `base_commit_mismatch` | The fetched ref is not a commit, or not `base_commit` |
| `source_unavailable` | The Git Server stayed unreachable or answered 5xx until the three attempts ran out |
| `destination_conflict`, `operation_failed`, `interrupted` | As before: a destination clash, a local failure, an attempt terminated or found stopped, with stop evidence |

Each workload command is recorded, with its attempt lock, before dispatch, and its end (Git exiting,
the kill after the stop grace or deadline, or settlement after a restart) is classified only by its
stop evidence, see [Node](node.md#execution-and-recovery):

- an exit code with stop evidence continues to the next step or to verification;
- a signal termination with stop evidence fails as `interrupted` and keeps the directory;
- an attempt a restarted Node finds stopped, without an exit status, fails as `interrupted` and keeps
  the directory too;
- missing stop evidence stays `Unknown`.

Missing process evidence never justifies another attempt. A known failure keeps its directory; a retry
needs new operation and execution identities and gets a different directory. Terminal replay always
uses the original durable result and event, without re-inspecting or fetching again.

## Records

Clone records live in the Node database:

- `accept_clone` atomically stores the original command, a generated repository identity, the root and
  path, and the initial reservation; it never creates a directory. Replaying the same input returns the
  original target; changed input or reuse across families fails. A destination cannot overlap any
  active worktree target or retained clone target, and a failure never releases its reservation.
- Progress records `Reserved`, `DirectoryCreated`, `Fetching` (with the durable attempt count) or
  `Dispatched` (with the current workload step). Directory identity is frozen after creation and
  `Unknown` keeps its phase. Each workload step records at most one attempt, and earlier attempts
  must be settled first.
- The terminal result and its event commit in one transaction and must match the original input,
  resource and path. Success requires zero exits for every workload step; a signal termination or a
  stop without an exit status backs only `interrupted`.
- Schema v10 adds `workspace_branch_heads`, the last value of each own Workspace branch a checkout is
  known to have on the Git Server, which [ora-git](ora-git.md#pushing) declares when it pushes. Clone,
  session and delivery records written before the snapshot input are not migrated: their inputs no
  longer decode, and their Workspaces are recreated.
- Queries never acknowledge; `pending_events` spans every family, and an acknowledgement removes only
  the delivery record while results, input and the reservation survive.
