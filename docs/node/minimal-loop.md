# Node minimal loop: clone a specified repository and branch

English | [中文](minimal-loop.zh.md)

## Current direction

As of 2026-09-18, the minimal loop changes from creating/deleting task worktrees under an existing
Main Workspace to **cloning a specified repository at a specified branch**. The local
Desktop–Controller–Node split remains: the caller specifies the repository and branch, Controller
coordinates, Node performs the clone in its execution environment, and the caller can observe the result.

This records the overall direction. Protocol, durable clone records and the Linux managed execution
API are implemented; see [clone deployment and recovery](repository-clone.md). The end-to-end loop is not connected yet.

## Existing foundations and gaps

- The [standalone Node](runtime.md) provides startup recovery, shutdown and injected data paths;
  Linux host/guardian can run managed Git.
- [Durable Worktree execution](persistence/worktree-execution.md) remains implemented, but is no longer
  the first end-to-end objective.
- Clone has independent protocol results, capability declarations, storage and managed execution.
  It does not require an existing Main Workspace or disguise acquisition as EnsureWorktree.
- Node-facing IPC, Controller coordination and the new Client entry are not connected. Existing passing
  tests do not establish a clone loop.

Stable execution identities, durable responsibility before dispatch, process handoff, queryable results
and acknowledgement after durable takeover remain reliability principles. Trust infrastructure and Strong
containment remain deferred; private Git access uses trusted, noninteractive Node deployment credentials.
Existing Backend writers, Worktree records and filesystem layouts remain unchanged.

## Approved boundaries and remaining design

| Topic           | Approved policy / remaining work                                                                         |
| --------------- | -------------------------------------------------------------------------------------------------------- |
| Input           | HTTPS and explicit SSH, deployment credentials, explicit branch; return the actual fetched commit        |
| Local resources | Node allocates an exclusive destination under an injected root; preserve failed/unknown residue          |
| Git scope       | Full single-branch history and checkout; hooks disabled, no recursive submodules or LFS downloads        |
| Coordination    | Durable original-execution replay is implemented; local IPC, Controller takeover and Client entry remain |

The accepted discussion about disabling Worktree-management hooks and repository-wide gates for unknown
process cleanup has not been implemented. It is neither current code behavior nor a complete clone failure model.

## Specification entry

The [clone root decision](../../specs/decisions/node/repository/0-clone-selected-repository-branch.md)
and [minimal execution contract](../../specs/decisions/node/repository/20260918-minimal-clone-execution-contract.md)
were approved on 2026-09-18, confirming scope, input, destination, content and recovery policies. Core verification obligations
include real HTTPS/SSH clone, Node-kill recovery and terminal-write failure evidence. Controller/Client
acceptance remains missing. There is no user-directory migration, IPC integration or Backend writer cutover.
