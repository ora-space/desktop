# Rootless guardian and trusted local management

English | [中文](guardian.zh.md)

The current deployment trusts the local Node and management programs, as explicitly directed by Eric.
Guardian management uses no secret tokens, signing keys or Controller authorization. Private paths and
kernel peer UID checks prevent accidental cross-user access; they do not isolate malicious same-UID code.
Host instance identities and durable epochs prevent stale cooperating instances from acting after takeover.

## Independent ownership

The caller explicitly provides a dedicated state directory and trusted executable to
[HostState](host/storage.md). No HOME inference, root helper or service installation is required.
Host creation intent and a consumed launch record commit before exec. A dedicated socketpair delivers
bootstrap identity; the original exclusive scope flock is inherited without unlocking and reacquiring.
The child enters its own session, clears its environment and closes unintended descriptors at exec.
Only --bootstrap appears in argv. No parent-death or Drop-kill policy is installed.

The guardian validates the inherited open description, its scope inode, private paths and local filesystem.
Only it initializes guardian.sqlite with WAL/FULL, then publishes control.sock, events.sock and io.sock.
Ready follows initialization; it does not prove Run startup or completed cleanup. Launch errors,
lost replies and guardian death never authorize another guardian for the same Scope. Existing journals,
locks and endpoint paths are preserved; partial initialization is not automatically repaired.

## Identity-based takeover

GuardianManagement::bind receives the binding committed under HostState ownership. The trusted caller
must cooperate with that owner, not invent higher epochs. Higher bindings commit before acknowledgement;
same-binding retries are idempotent, lower epochs and same-epoch conflicting instances are rejected.
A host session contains the public host binding only, not a bearer credential.

Requests are checked under the execution mutex against current journal authority, including requests
queued before takeover. Scope lock ownership lasts until remaining workers close SQLite. An unfinished
transaction or storage failure cannot produce a successful takeover acknowledgement. Ready discovery
is independent of current host-session inspection.

Messages use bounded length-prefixed MessagePack (16 KiB, depth 16), version 3. Each exchange has a
five-second I/O deadline and channel workers are independently bounded. The guardian additionally
drives lifecycle reconciliation every 50 ms, independently of host connections.

## Trusted local Run loop

After Ready and bind, use GuardianRuns::execute with protocol-owned GuardianRunOperation values:

- Start requires a never-reused RunId, exact RunSpec and explicit host_disconnect: KeepRunning.
  The guardian commits the specification and an uncertain initial snapshot before exec. Same-ID,
  same-spec retries return the original attempt; changed specs conflict. Even proven exec failures
  cannot be retried under that ID. Environment values are stored verbatim in the private journal,
  not logged; do not treat the journal as a redacted artifact.
- Query returns the journaled snapshot, separating direct exit from best-effort cleanup. A lost
  response is not evidence of non-acceptance. Storage errors after exec retain the original attempt.
- Stop requests force cleanup for one Run; Close durably seals the scope before force cleanup.
  Repeating these requests is safe. Closure does not prevent querying or replaying existing attempts.
  Signal delivery is not a terminal result; poll Query/Scope for cleanup evidence.
- Output reads an independent stdout/stderr prefix through io.sock. Capture is explicit in RunSpec:
  at most 1 MiB per stream, 4096 bytes per read and 64 accepted attempts per scope, including terminal
  attempts. Admission rejects excess limits; no automatic record deletion or output eviction occurs.
  Truncation and EOF are explicit. Bytes remain volatile and disappear with the guardian.
- Control carries Start/Query/Stop/Close/Scope; events.sock currently has no subscriptions.
  All Run requests check the live public host binding inside the takeover execution mutex.
  stdin is closed. Host disconnect means KeepRunning, not an inferred lease expiration.

Workloads use the existing LinuxBestEffort adapter and its pidfd observations, in separate sessions
with unintended descriptors closed at exec. Descendants escaping before discovery may survive.
Guardian SIGKILL can leave workloads alive: old numeric PIDs never become signal authority, stored
Running facts are historical rather than current evidence, and neither guardian nor Run is respawned.
The host and [standalone Node](../node/runtime.md) now compose this loop for managed Git;
Controller IPC and existing Backend business launchers remain outside this integration.

## Existing files and versions

RunSpec now supports explicit `TerminateOnOwnerExit` liveness. The Linux adapter pins the observed
owner before exec and force-cleans the Run when that pidfd exits, independently of host connectivity.
The numeric owner identity is never signaled and is only a cleanup trigger, not a resource-handoff
certificate. Callers still query the original Run's cleanup. The default remains `Independent`;
old stored specifications without the field retain that policy. Wire v3 rejects old live peers;
keep their compatible manager and journals rather than replacing an original guardian.

The host journal uses version 6 and the guardian journal version 4, each with its own Run ledger. Neither contains credential columns. Host recovery migrates
the exact v1/v2/v3/v4/v5 layouts transactionally, retaining original scopes, epochs, lock inodes and consumed
launch attempts. Version-2 hosts with existing Scope directories are rejected before migration so
compatible old binaries retain the original tokens and epoch. Use that compatible host or a separate
new state directory; do not delete legacy scopes to force an upgrade. Removed historical token values
are not promised to be securely erased from SQLite
free pages or backups. Unknown layouts fail without repair.

Live older guardians require the older token protocol and are incompatible with the new client.
Use their compatible management version where needed; do not rewrite their journals, replace their
binary while responsible, or restart old scopes. This increment does not adopt running workloads.

## Verification and limits

Real-app tests cover independent sessions, launcher SIGKILL survival and takeover, original-instance
discovery, failed-exec deduplication, wrong scope/UID, inherited lock qualification, existing-journal
refusal, all three stale channels, late bytes and lost takeover replies. Runtime tests cover queued
execution checks, persistence failure and lock lifetime. Host tests cover exact v1/v2 migration.
Run app tests cover exact replay, exit/output, lost Start replies, pre/post-exec storage failure,
host SIGKILL takeover with Run discovery from the host journal, force/close and guardian SIGKILL with a surviving workload that holds no scope lock.

Controller authorization and leases are deferred. Host coordination and durable query projections are implemented;
[the host app](host/service.md) exposes them over local IPC. Remaining work includes
stdin, durable output, plugin integration and guardian-death recovery. Node/Git is integrated. Strong containment,
service-manager survival, physical power loss and other-platform support are not established.
