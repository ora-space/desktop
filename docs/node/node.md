# Node

English | [中文](node.zh.md)

`ora-node` (`apps/ora-node`) is the Linux executable that runs work inside an execution environment:
in Cloud mode, one Node runs in each Workspace sandbox. It owns one explicitly configured Node
database, executes clones, plugin installations, Agent sessions and Revision deliveries, recovers
unfinished executions after a restart, and runs every Git command itself as a direct child process,
see [Execution and recovery](#execution-and-recovery). It accepts commands only from its Controller
over the [control session](control-session.md); configuration files and stdin are not command
channels.

## Deployment

Build with `cargo build -p ora-node -p ora-reaper`, then run
`ora-node /absolute/path/node-config.json`. A minimal configuration:

```json
{
  "node": { "home_directory": "/home/alice/.ora/node", "identity": "Discover", "repositories": [] },
  "process": {
    "git_program": "/usr/bin/git",
    "environment": { "PATH": "/usr/bin:/bin", "HOME": "/home/alice" },
    "command_timeout_ms": 30000,
    "cleanup_timeout_ms": 5000,
    "shutdown_grace_ms": 2000
  },
  "timezone": "Asia/Shanghai",
  "recovery_interval_ms": 1000
}
```

Further sections enable capabilities: `clone` ([Repository clone](repository-clone.md)), `control`
([Control session](control-session.md)), `agent` ([Agent sessions](agent-sessions.md)), `plugins`
([Plugins](plugins.md)) and `git_access` ([Revisions](revisions.md#git-server-access-material)),
which clone, Revision delivery and [ora-git](ora-git.md) require; a Cloud-mode Node refuses to start
without it.
`process.workload_uid` runs Git workloads and agents as a separate user.

- `process.reaper_program` is the absolute path of the `ora-reaper` executable; without it the Node
  uses the `ora-reaper` installed next to its own executable.
- Two legacy `process` fields are still accepted. With `host_directory` (absolute, never containing
  the Node home), the Node deletes the retired process host's state directory at startup, but only a
  directory that contains `host.sqlite` and whose `host.lock` is free. `expected_uid` is ignored.
- `"Discover"` keeps an existing NodeId or generates the first one; `{ "Require": "id" }` enforces a
  registered identity. Each start generates a fresh NodeIncarnationId.
- The Node home is absolute and injected, never derived from `HOME` (the `HOME` in `environment`
  configures Git only). A new directory is private; release builds reject an existing non-private
  one, while debug builds skip permission-bit checks. A wrong owner or a symlink through a trusted
  path is rejected. The home must be on a local filesystem, because it holds the attempt locks.
- `node.repositories` holds explicit `RepositoryBinding`s (repository reference, Main Workspace
  identity and path, authorized root, worktree root) for [worktree execution](worktree-execution.md).
  Nothing is cloned or inferred from the cwd.

## Execution and recovery

The Node starts every Git command itself, as a direct child, with the configured executable and
environment. Each command leads its own process group, and with `process.workload_uid` Git that
touches a checkout drops to that user between fork and exec. Every command carries
`-c gc.auto=0 -c maintenance.auto=false`, so Git detaches no background maintenance.

At startup the Node starts one `ora-reaper` sidecar and registers every Git it runs with it; if the
Node dies, the reaper kills the registered process groups. A Node that cannot start the reaper does
not start, and when the reaper exits while the Node runs, the Node starts no further Git and exits
with an error. Agent session and plugin processes are not registered with the reaper, see
[Agent sessions](agent-sessions.md#record-order-and-crashes).

**Attempt locks.** Each command with side effects owns a lock file `<attempt-id>.lock` (mode `0600`)
in the private directory `<node.home_directory>/git-attempts/` (mode `0700`, owned by the Node's
user). The file exists before the attempt is recorded and before the command is dispatched; the Node
takes an exclusive `flock` on it, and Git and all its descendants inherit that open file. **Stop
evidence** is the Node taking that lock again without blocking. A lock that is still held means some
process of the tree is alive, so the command has not stopped; a missing lock file or a failed probe
is `Unknown`, never "stopped". Next to the lock file, `<attempt-id>.clue` holds the direct child's
pid, start time and boot id. It is not synced and only lets a restarted Node kill the process group,
after checking that the pid was not reused.

| Command | Attempt lock | Recorded in the Node database |
|---|---|---|
| Worktree mutations, the clone's checkout commands, Revision restore commands | Yes | Yes, before dispatch |
| Revision delivery, `ora-git` fetch and push, the clone's transport-repository commands | Yes | No: an interrupted one is prepared again |
| Read-only queries | No | No |

A recorded attempt is never dispatched again, so a lost result never authorizes a fresh attempt.
While the leftover lock of an unrecorded command is still held, no new unrecorded command starts.
Read-only queries still run in their own process group under the reaper.

**Output and deadlines.** stdin is closed; stdout and stderr are read into bounded memory and never
persisted. Output over the limit fails the command, and truncated, failed or missing output is never
parsed as complete Git facts. `command_timeout_ms` bounds local commands and `network_timeout_ms`
network ones. On a deadline, or once the grace of a Node stop has passed, the Node kills the whole
process group; after the direct child ends it kills the group once more, then waits up to
`cleanup_timeout_ms` for stop evidence. A command ends as exited (with a code), terminated (by a
signal) or unverified. An unverified command leaves its execution `Unknown` with its reservations
intact, while non-conflicting work can still proceed.

**Restart.** Before any business recovery the Node settles the locks its previous process left: it
kills the process group a clue names when the identity matches, then probes the lock. A recorded
attempt whose lock is free ended as "stopped without an exit status": a clone fails as `Interrupted`
with its directory retained (only a new execution may retry), a restore's stop counts as confirmed,
and a worktree execution goes on to inspect its resources (see
[Worktree execution](worktree-execution.md#recovery)). A lock that is still held stays `Unknown`; its
file is kept and probed again later. A recorded attempt without a lock file is `Unknown` too.

The guarantee is `BestEffort`, not strong containment. It does not cover a descendant that closes its
inherited descriptors and leaves the process group, a lock file held open by another process of the
same user (deployments without a separate workload user), or Git that keeps running after the Node
and the reaper died together, until the Node restarts.

`SIGTERM` and `SIGINT` close admission, give the active Git command its grace (`shutdown_grace_ms`),
then kill its process group. Shutdown settles pending attempts and reports failure rather than
claiming an unverified stop; business reconciliation may remain for the next start.

## Storage

The database is always `home_directory/ora-node.sqlite3`. It holds the business execution, resource
and event records and the recorded Git attempts; the attempt lock files live beside it in
`home_directory/git-attempts/`.

- Opening holds an exclusive OS lease on the sibling `ora-node.sqlite3.lock` for the database's
  lifetime, never on the database file, whose SQLite locks would otherwise collide with a whole-file
  lease on macOS and Windows. The lease excludes another Node owner but does not prove an older Git
  process stopped.
- SQLite uses its default rollback journal with `synchronous=FULL`. The database has application id
  `0x4f52414e`. Opening checks identity, integrity, exact table and index definitions and foreign
  keys before anything else; empty files, foreign databases, unsupported versions, directories and
  corrupt databases are rejected and never rebuilt.
- Older exact schemas migrate transactionally, preserving executions, results and pending events;
  older binaries refuse newer versions instead of reading them.

| Version | Adds |
|---|---|
| 1 | `node_metadata`, `executions`, `resources`, `outbox` (Worktree executions) |
| 2 | `managed_executions`, `process_attempts`: process associations |
| 3 | `execution_identities`, `clone_executions`, `clone_outbox`, `process_outcomes` |
| 4 | `controller_binding`, `execution_controllers`: the deployment-bound ControllerId |
| 5 | `process_terminations`: attempts a signal ended after confirmed cleanup |
| 6 | Runtime bindings, enforcement and execution permits |
| 7 | `plugin_executions`, `plugin_outbox` |
| 8 | `node_executions`, `execution_events`, `session_commands`: Agent sessions |
| 9 | `revision_deliveries`, `revision_outbox` |
| 10 | `workspace_branch_heads`: the own Workspace branch values `ora-git` pushes declare |
| 11 | `session_restores`, `restore_process_attempts`: restore admission, original identities and stop clocks |
| 12 | `scheduled_artifact_deliveries`: scheduled-artifact uploads; the terminal-event outbox covers every execution family |
| 13 | `process_stops`: attempts that stopped without an exit status, exclusive with `process_outcomes` and `process_terminations` |

Every execution family shares `execution_identities`, which prevents key collisions across families.
Completion commits resource facts, the terminal result and its original event together; status reads
never acknowledge. An acknowledgement must match Node, operation, execution and sequence and removes
only the delivery record, while results and deduplication survive. Migration never invents process
evidence for legacy direct-Git executions, so such an in-flight execution stays blocked. The upgrade
to version 13 settles every attempt the retired process host left unsettled as stopped without an
exit status, and a restore attempt in that state has its stop confirmed: deployment ends the old
Node, host and guardian processes before this Node starts. A stored attempt keeps the command's
directory and arguments, never its environment, which may hold deployment secrets.

`WriteGuard` exposes transaction failure points for real SQLite fault tests; `cargo test -p
ora-node-db -p ora-node` runs them.

Restore Git admission and its original command intent commit together. A session terminal requires original stop evidence; bound expiry retains named residue and never deletes identity. Pre-journal restores that were still in flight remain `legacy_unknown` and keep checkout and runtime admission fenced; completed old terminals are retained as history without fencing later admissions. See [Revision recovery](revisions.md).
