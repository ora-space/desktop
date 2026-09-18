# Standalone Node runtime

English | [中文](runtime.zh.md)

> The current objective is [cloning a specified repository and branch](minimal-loop.md). The runtime below
> describes existing Worktree execution, not an available clone command or completed new loop.

The Linux `ora-node` executable owns one explicitly configured Node database, recovers pending
Worktree executions and handles normal shutdown. It installs **no Controller IPC**, accepts no new
commands from stdin or files, and does not switch existing Backend writers. Embedding callers use
`Node::open(config, process_config, shutdown)` and the existing typed Node methods.

## Deployment

Build `cargo build -p ora-node -p ora-process-host -p ora-process-guardian`. Deploy/start the
[independent process host](../process/host/service.md), then run:

```text
ora-node /absolute/path/node-config.json
```

For an idle Node without registered repositories, the configuration can be:

```json
{
  "node": {
    "home_directory": "/home/alice/.ora/node",
    "identity": "Discover",
    "repositories": []
  },
  "process": {
    "host_directory": "/home/alice/.ora/process",
    "expected_uid": 1000,
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

Replace the paths, UID and timezone for the deployment. `Discover` retains an existing NodeId or
generates the first one; `{"Require":"registered-node-id"}` enforces a registered identity.
Repository bindings use `RepositoryBinding`: repository reference, existing Main Workspace identity/path,
authorized root and worktree root. No repository is cloned or inferred from cwd.

Node data is always `home_directory/ora-node.sqlite3`. Both state directories are absolute and injected;
neither is selected from HOME. The environment's HOME above configures Git only. New Node directories
are private on Unix; existing nonprivate directories, symlinks through trusted paths and aliases of
host state are rejected rather than chmod'ed, overwritten or repurposed. Existing v1 Node databases
follow the [storage migration](persistence/storage.md).

## Execution and recovery

All Git invocations use the configured executable/environment through host and guardian. Read-only
preflight runs have no business mutation association and share bounded read scopes. Before each mutation,
Node commits its execution/host/Scope/Run association in its own database; the host and guardian
retain their own intent and facts. A lost reply does not authorize a fresh attempt.

On recovery, Node seals each unfinished original mutation Scope and waits for observed closure before
examining or repairing its resources. An unavailable guardian or unverified cleanup leaves that execution
Unknown and its resource reservation intact. Other nonconflicting work can still be submitted through
the library. A migrated legacy in-flight direct-Git record without process evidence remains blocked;
neither upgrading nor acquiring the database lock proves its old process stopped.

Owned branch-only creation at the frozen base enters durable `CleanupCreation`, cleans only its owned
residual and retries once per recovery pass with the original input and base commit. Other checkout
occupancy, changed branch content and unexplained nonempty residuals are not forcibly repaired.
Valid owned worktrees with new commits complete creation without resetting those commits.

Guardian pins the requesting Node's observed process identity before exec. Node exit triggers Run cleanup
even if host is disconnected. This is rootless **BestEffort**, not strong containment: an escaped descendant
or guardian death can prevent verified cleanup. Owner liveness is only a cleanup trigger, not authorization
to recover resources. No authentication token or lease is introduced.

SIGTERM/SIGINT close new-work admission, allow the active Git command its configured grace, then request
cleanup. Shutdown verifies pending mutation scopes and the read scope, reporting failure rather than
claiming unverified cleanup. Business reconciliation may remain pending for the next startup. Output is
bounded and volatile; truncated, failed or missing output is not parsed as complete Git facts.

## Verification and next boundary

`cargo test -p ora-node --test standalone` exercises the actual Node, host and guardian executables with
real SQLite and a blocking Git hook. Build the three executables first for a focused run. Tests cover
Node SIGKILL followed by immediate restart, guardian cleanup without a replacement Node, guardian loss
with continued unrelated work, graceful timeout and data-directory isolation. Unit tests cover local
recovery failures, frozen bases, result persistence, ownership and replay. Run `cargo test -p ora-node-db`
for migration and process-association fault tests.

Controller delivery/result takeover, Backend switching, non-Linux managed adapters, Strong containment,
persistent output, stdin and process-history/guardian retirement remain outside this slice. Existing
process history is retained; this is not a claim that the full process ADR blueprint is complete.
