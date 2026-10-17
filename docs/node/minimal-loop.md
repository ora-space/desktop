# Node minimal loop: clone a specified repository and branch

English | [中文](minimal-loop.zh.md)

## Current direction

As of 2026-09-18, the minimal loop changes from creating/deleting task worktrees under an existing
Main Workspace to **cloning a specified repository at a specified branch**. The caller specifies the
repository and branch, Controller coordinates, Node performs the clone in its execution environment, and
the caller can observe the result. Requests enter through Cloud (the tenant clone API and Workspace
operations), the Controller dials out to Cloud to claim them, and local integration runs in the cluster
repository's Compose stack. The non-production local minicloud, the first Client from 2026-09-20, had the
browser call an interface hosted by the Controller, which contradicts a Controller that only dials out;
it was removed on 2026-09-28.

This records the overall direction. Protocol, durable clone records and the Linux managed execution
API are implemented; see [clone deployment and recovery](repository-clone.md). The end-to-end loop is not connected yet.

## Existing foundations and gaps

- The [standalone Node](runtime.md) provides startup recovery, shutdown and injected data paths;
  Linux host/guardian can run managed Git.
- [Durable Worktree execution](persistence/worktree-execution.md) remains implemented, but is no longer
  the first end-to-end objective.
- Clone has independent protocol results, capability declarations, storage and managed execution.
  It does not require an existing Main Workspace or disguise acquisition as EnsureWorktree.
- [Local Node IPC](local-ipc.md) and [Controller durable coordination](../controller/local-runtime.md)
  are connected, with independent-process clone, pre-commit kill and lost-Ack recovery tests. The
  Controller's cloud form receives clones accepted by Cloud over the
  [Controller–Cloud contract](../protocols/controller-cloud-contract.md).

Stable execution identities, durable responsibility before dispatch, process handoff, queryable results
and acknowledgement after durable takeover remain reliability principles. Trust infrastructure and Strong
containment remain deferred; clones read only from the Git Server with the Node's git-access identity, never deployment credentials.
Existing Backend writers, Worktree records and filesystem layouts remain unchanged.

## Approved boundaries and remaining design

| Topic           | Approved policy / remaining work                                                                           |
| --------------- | ---------------------------------------------------------------------------------------------------------- |
| Input           | Git Server snapshot over HTTPS with the git-access identity, no SSH or deployment credentials; base commit |
| Local resources | Node allocates an exclusive destination under an injected root; preserve failed/unknown residue            |
| Git scope       | Full single-branch history and checkout; hooks disabled, no recursive submodules or LFS downloads          |
| Coordination    | Local IPC, durable Controller takeover, original-execution replay and intake through Cloud are implemented |

The accepted discussion about disabling Worktree-management hooks and repository-wide gates for unknown
process cleanup has not been implemented. It is neither current code behavior nor a complete clone failure model.

## Specification entry

The [clone root decision](../../specs/decisions/node/repository/0-clone-selected-repository-branch.md)
and [minimal execution contract](../../specs/decisions/node/repository/20260918-minimal-clone-execution-contract.md)
were approved on 2026-09-18, confirming scope, input, destination, content and recovery policies. Core verification obligations
include real HTTPS/SSH clone, Node-kill recovery, terminal-write failure and Controller lost-Ack recovery evidence.
The end-to-end path through Cloud has no automated test yet. Additional Node/Controller fault
combinations remain tracked in their own core cases. There is no user-directory migration or Backend writer cutover.
