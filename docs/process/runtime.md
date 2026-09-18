# Process runtime implementation status

English | [中文](runtime.zh.md)

The [approved process ADRs](../../specs/decisions/node/process/README.md) are being implemented incrementally.
The current increment connects the in-memory lifecycle kernel and
[rootless Linux best-effort adapter](linux/rootless.md) through a
[trusted local guardian Run loop](guardian.md) and [standalone Node Git execution](../node/runtime.md).
Existing Backend business launchers have not been switched over.
Existing `ora-process`, `ora-reaper`, Git and plugin entry points are unchanged.

Linux also has an independent [helper deployment preflight and authenticated inspection service](linux/helper.md). Its checks
do not enable workload launch or constitute a platform adapter. A low-level pre-exec launch gate now
exists for trusted helper code; it is not exposed over IPC and awaits privileged acceptance testing.

## Ownership and behavior

- `ora-process-protocol` owns local domain types: run identity, exact launch specification, containment
  selection, stop intent, direct exit facts and cleanup evidence, plus the helper's inspection-only
  wire types and bounded MessagePack guardian discovery, management and Run messages.
- `ora-process-client` depends on protocol types, not runtime or SQLite. The thin Linux
  `ora-process-guardian` app delegates bootstrap, host takeover and durable Run operations to runtime.
- `ora-process-runtime::ScopeRuntime<P>` owns one scope's admission, run records and stop deadlines.
  `Platform` supplies verified capabilities, creation-time containment, observations and per-run signals.
  Linux has a rootless adapter; controlled tests also inject platform facts through this boundary.
- Scope creation freezes the actual guarantee. Required strong containment is rejected when unavailable;
  explicitly requested best-effort containment is never silently promoted to strong containment.
- Replaying a RunId with identical parameters returns its current facts; changed parameters conflict.
  Unknown launches are never retried. Proven non-starts are currently replayed too, not resumed.
- Closing seals admission before scheduling cleanup. Stopping one run leaves its scope and peers open.
  Wait, notify-then-wait and force requests only tighten existing deadlines or escalate actions.
- Direct exit and descendant cleanup are separate. The cleanup policy notifies descendants and forces
  them after its explicit grace period; wait-for-all leaves them managed until they exit or are stopped.
- Signal delivery alone never proves cleanup. Failed observations and signals retain responsibility.
  Direct running/exit evidence confirms an uncertain launch without another spawn. Exit status may
  become more precise but never less precise; contradictory launch/exit observations remain blocked.

## Caller obligations and remaining work

Calls are serialized through mutable ownership. The caller must drive `reconcile` with monotonic
`Instant` values; this kernel has no background task, timer, retry backoff or Drop-based cleanup.
Platform methods must be bounded and must retain stable attempt identities, including after uncertain
spawn outcomes. Dropping the kernel does not provide crash recovery.

Linux now supports [bounded volatile result capture](linux/rootless.md#bounded-result-capture),
with independent readers and per-run limits; pipe EOF is separate from process cleanup.
Host creation intent now has an opt-in [durable journal](host/storage.md) under an explicitly supplied
dedicated directory and [independent guardian bootstrap/Ready discovery](guardian.md).
Guardian-side durable Run acceptance and polling/output/force-stop now work for trusted local callers.
Authorization and leases are deferred. Host Run intent, restart enumeration, automatic coordination, durable query projection and [the host app](host/service.md) are implemented. Remaining
platform adapters, full I/O, runtime recovery, resource handoff and production integration remain unimplemented.
This increment does not complete implementation phase 1 or prove any OS-level containment guarantee.

## Guardian bootstrap foundation

The workspace now uses bundled `rusqlite` 0.40.2 / SQLite 3.53.2, including the
[WAL-reset fix](https://sqlite.org/wal.html). `ora-db` tests query `sqlite_version()` and
`sqlite_source_id()` through a real pooled connection and require the linked mainline engine to be
at least 3.51.3. They also cover committed versus uncommitted visibility across connections,
rollback, checkpoint and reopening a file-backed database. This is a dependency prerequisite,
not guardian crash-durability evidence; the existing application database schema and its
`synchronous=NORMAL` policy are unchanged. Host and guardian journals independently enable `FULL`.

The approved [rootless guardian bootstrap decision](../../specs/decisions/node/process/recovery/20260917-rootless-guardian-bootstrap-and-reconnect.md)
now has its first building block: `ora_utils::fs::LinuxFileLock`. It accepts an already opened
regular file, attempts exclusive acquisition without waiting, and returns `WouldBlock` for contention.
Cloning duplicates the locked open file description; `into_file()` transfers it for explicit child
handoff without releasing/reacquiring the lock. Descriptors default to close-on-exec.
An unrelated concurrent fork can still hold a temporary copy before exec; closing the local owner
does not promise immediate reacquisition. Callers must observe actual lock acquisition.

Drop only closes a descriptor: it deliberately does not issue an explicit unlock, which could also
release a child's shared lock. This follows Linux's [flock lifetime semantics](https://man7.org/linux/man-pages/man2/flock.2.html).
The caller must preserve the inode, resolve trusted paths and qualify the actual local filesystem.
This primitive neither creates nor deletes files, proves data durability, nor proves workload cleanup.
`try_acquire` may lock an unlocked file; `adopt_inherited` instead requires an already-held exclusive
flock on that open description. The guardian separately verifies scope/path binding. Same-user
adversaries, network filesystems and other platforms are not certified.

`cargo test -p ora-utils --test linux_file_lock` verifies contention, duplicate lifetime, unchanged
file contents, close-on-exec (including an explicit pre-exec barrier), and an exec'd holder retaining exclusion after its launcher is killed
externally. The surviving holder is identified and killed through a pidfd; acquisition must become
possible again. The child test fixture is not a guardian or host implementation. State-directory
admission and persistent creation intent now have a [host-owned implementation](host/storage.md),
with independent external-kill tests of the journal owner. [Real-app bootstrap tests](guardian.md)
now additionally verify inherited qualification, journal-before-Ready and original-instance discovery
after launcher death. Durable host takeover fences both session inspections and local Run execution.
Controller authorization and stdin remain deferred; no network Run endpoint is exposed.

## Verification

Run `cargo test -p ora-process-runtime` and
`cargo clippy -p ora-process-protocol -p ora-process-runtime --all-targets -- -D warnings`.
Controlled integration tests exercise the public runtime interface with injected platform facts and
time. Rootless Linux tests additionally exercise real children and descendants with readiness
handshakes and bounded polling. Neither mutates the test runner's environment. Evidence remains
`Partial` in the [core case index](../../specs/test-cases/node/process/README.md); crashes, persistence,
exhaustive identity races and strong platform permission boundaries still require direct verification.
