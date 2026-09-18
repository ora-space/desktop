# Rootless Linux process tracking

English | [中文](rootless.zh.md)

`ora_process_runtime::LinuxBestEffort` provides the first real Linux
adapter for `ScopeRuntime`. It needs no root, sudo, privileged helper, cgroup delegation,
service installation or separate workload account. Workloads retain the caller's identity.
The privileged [helper route](helper.md) is preserved but is not the current priority.

## Admission and ownership

The adapter advertises only `BestEffortOnly`. `RequireStrong` is rejected before launch;
`PreferStrong` selects `BestEffort` before accepting work. Completion is
`BestEffortComplete`, never `ConfirmedQuiescence`.

Construction checks readable procfs, pidfd operations and `waitid(P_PIDFD)` support, and
rejects automatic child reaping via `SIGCHLD` ignore or `SA_NOCLDWAIT`. Restricted kernels,
procfs mounts or syscall policies can reject construction; rootless does not mean every
container or gVisor configuration is supported. Procfs must describe the caller's PID namespace.

Every run starts a new session before exec. Its environment is exactly `RunSpec.env`, not an
implicit inheritance of the host environment. `with_discarded_io()` discards all three standard
streams and rejects capture requests before exec. `with_bounded_output()` additionally accepts
`RunSpec.output = OutputPolicy::Capture { stdout_limit, stderr_limit }`. Stdin remains closed.

## Bounded result capture

Limits are explicit per-stream byte counts and part of exact RunSpec replay identity. Zero accepts
only empty output; exactly the limit is not overflow. The default RunSpec policy remains `Discard`.
Each captured pipe has an independent reader that progresses without output consumers or reconcile.
Only the bounded prefix is retained; after overflow it continues draining without growing retention.
Overflow or reader/setup failure requests force cleanup at the next `reconcile`, even when the
descendant policy is `WaitForAll`. The caller must continue driving reconciliation; reader threads
do not independently own process-control authority.

`ScopeRuntime::read_output(run, stream, offset, max_bytes)` copies at most the requested bytes,
without consuming them. It reports retained length, sticky truncation and `Open` / `Eof` / `Failed`.
Offsets past the retained prefix are errors, not silent gaps. Query both streams: an EOF with
truncation is incomplete, and a read failure is not EOF. Bytes are volatile, never a durable offset
or a plugin-session recovery promise. Unknown, non-started and discarded runs have no capture.

Direct exit, tracked cleanup and pipe EOF remain independent. A descendant can hold a pipe after
direct exit; an escaped descendant may hold it even after best-effort cleanup. Closing stdout does
not imply process exit. Output remains readable after cleanup until the scope/adapter is dropped.
Drop cancels readers without waiting for pipe writers. This slice uses two reader threads per
captured run and retains bounded data per run, not an aggregate scope quota or retirement policy.
It does not implement plugin backpressure, log rotation, stdin, persistence or guardian handoff.

The caller must drive `ScopeRuntime::reconcile` and exclusively own child reaping. No other
thread or signal handler may reap these children or enable automatic reaping while tracking.
The direct child remains unreaped until tracked cleanup completes, preserving the session ID's
identity even after direct exit. A post-launch pidfd acquisition failure keeps ownership and
reports an uncertain launch; later observations retry acquisition, never launch a duplicate.

## Discovery, stopping and evidence

- Scan the original session for members, pin their proc directories during pidfd acquisition,
  and retain captured pidfds when members later detach or are reparented. Do not infer ownership
  from historical PPIDs or broadcast signals to a numeric process group.
- Notify sends `SIGTERM`; force sends `SIGKILL`. Stop intent persists so later discoveries receive
  it too. Failed discovery does not prevent signaling already captured members.
- Signal through pidfds. Only the exclusively owned, unreaped direct child may use `Child::kill`
  as a force fallback if pidfd acquisition/signaling fails; that failure still remains visible.
- Report best-effort completion only when direct exit and all captured exits preceded a fresh,
  successful scan that found no new identity, and all captured members are still exited. Reap the
  direct child only then. Scan/signaling errors and live captured members prevent completion.
- Dropping the adapter attempts force cleanup and starts a direct-child reaper. This is not a
  cleanup certificate and does not survive a crash or `SIGKILL` of the owner.

Descendants that create another session **before discovery**, or new descendants born outside the
original session, may escape tracking. PID handles prevent redirecting signals to a recycled PID;
they do not make discovery exhaustive or prevent escape. This is intentionally best effort, not
a security boundary against same-user workloads.

## Verification and remaining work

Run `cargo test -p ora-process-runtime --test linux_best_effort` and
`cargo test -p ora-utils --test linux_process` as an ordinary Linux user. Real-process tests cover
admission, replay without duplicate execution, direct exit with surviving descendants, run isolation,
notify/force escalation, captured-member `setsid`, and Drop's direct-child cleanup. Utility tests cover
pidfd exit/reaping, stale proc observations and non-UTF-8 process names. They do not force actual
numeric PID reuse or exhaust descriptors to verify post-launch acquisition recovery.

`cargo test -p ora-process-runtime --test linux_output` covers independent binary streams, exact
limits, replay conflicts, offset reads, dual-stream output beyond pipe capacity without a consumer,
overflow termination and isolation, descendant-held pipes and EOF before exit. Shared reader tests
are `cargo test -p ora-utils --test pipe_capture`; they cover zero/exact/excess capacity, offsets and
cancellation with a live writer. Thread/descriptor exhaustion and read-error recovery remain gaps.

Durable host/guardian ownership, crash recovery, I/O handoff and production entry points remain
unimplemented. See the [runtime status](../runtime.md). No strong-containment ADR is completed
by these tests, and Linux results do not establish Windows/macOS support.
