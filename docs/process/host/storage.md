# Process host scope and Run intent journal

English | [中文](storage.zh.md)

Linux `ora_process_runtime::HostState` owns durable Scope/Run intent and one-shot
[independent guardian bootstrap](../guardian.md). The [host app](service.md) composes it with automatic coordination. It implements part of the approved
[guardian bootstrap decision](../../../specs/decisions/node/process/recovery/20260917-rootless-guardian-bootstrap-and-reconnect.md).
It requires no root, helper, cgroup delegation or service installation. Existing Backend Git/plugin entry
points and application database policy are unchanged.

## Explicit location and ownership

The caller supplies an absolute, dedicated `state_dir`. `HostState` never reads HOME or business cwd.
Deployment injects the host directory separately from Node data. The
[standalone Node](../../node/runtime.md) receives this host path explicitly for managed Git.

- `HostState::create(&state_dir)` requires an absent directory, including rejecting an existing empty
  directory. It creates only `host.lock`, `host.sqlite`, SQLite sidecars and `scopes/`.
- `HostState::recover(&state_dir)` requires the existing stable lock, database and scopes directory.
  Missing files, unknown root entries, malformed identities and incompatible journals fail without
  resetting state. It never retries creation after a recovery error.
- Directories are owner-private; files are owner-private regular inodes with one hard link.
  Symlinks and group/other-writable ancestors are rejected using shared `ora-utils::path` checks.
  Existing permissions are not changed. Root and the selected UID remain trusted; this is not
  isolation from malicious same-UID code.
- The initial filesystem allowlist is the ext family, XFS and Btrfs. Network, memory, overlay and
  unknown filesystems are rejected. Filesystem classification alone does not certify mount options,
  storage hardware or power-loss behavior. Local tests currently exercise ext-family storage only.
- The full canonical Scope ID and `control.sock` suffix must fit Linux's pathname socket limit.
  Long paths are rejected, not shortened or redirected. A group-writable checkout or `/tmp` is not
  a supported state parent.

Creation is intentionally fail-closed: interrupted initialization can leave a partial dedicated
directory, which is preserved and rejected on recovery. Automatic repair is not provided; only the
exact version-1 through version-5 journals have the migrations described below.
Do not delete lock files, replace the directory while owned, or remove records to bypass a failure.

## Durable facts, not launch authority

The host holds the original `host.lock` until its SQLite connection closes. Recovery first takes that
lock nonblockingly, then performs a read-only journal compatibility check, and only then commits a
new host incarnation. Contention is an error, not permission to replace a lock or endpoint.

The version-6 journal identifies itself with application ID `0x4f524148` and `user_version=6`, and
checks its exact schema, integrity and persisted identities. It stores a positive host epoch with a
host instance ID, plus each Scope's original guardian instance, creating host binding and
`intent_recorded` phase. Epoch overflow is rejected; recovery never rewrites an intent's creating host.
Existing scope directories must have canonical IDs, private directory metadata and matching host
intent. Their internal journals and endpoints are not inspected or managed by this slice.

`record_scope_intent(scope)` commits a new original intent or returns the existing one unchanged.
`scope_intent(scope)` queries that responsibility; absence is not permission to recreate a guardian.
This registration call produces no Scope directory, guardian journal, process, credential or Ready fact.
A pre-existing Scope path without an intent blocks registration and is preserved.

`start_guardian(scope, executable)` requires that intent and an explicitly supplied trusted executable.
It first commits a `launch_unknown` record, then creates the private Scope directory,
acquires its lock and execs the guardian. Every subsequent error or cancellation consumes the attempt;
even a proven exec failure cannot authorize a second launch. `guardian_access(scope)` restores the
original discovery material after host recovery; it neither launches nor transfers control authority.
Guardian readiness is queried separately through `ora-process-client`.

Recovery accepts the exact version-1 intent-only schema and atomically adds the launch table while
advancing host identity. Original intents, paths and lock inode remain unchanged. That old version
could not launch guardians; it has no launch records to infer or backfill. The exact version-2 schema is migrated only when no Scope directory exists. Existing legacy scopes
block migration before any authority write or credential removal; their guardian may still need the
original token protocol. Keep a compatible old host for them, or use a separate new state directory;
never delete a Scope to bypass this check. For eligible journals, the obsolete credential column is removed while all
consumed launch attempts are preserved. Historical SQLite free pages and backups are not securely
erased. Unknown schemas fail closed, and old binaries reject version 6 rather than resetting it. Host recovery never writes guardian.sqlite.

The journal independently enables and verifies WAL plus `synchronous=FULL`; the linked SQLite mainline
version must include the WAL-reset fix (at least 3.51.3). Transactions commit before returning new
facts; containing directory entries are also synced. A filesystem error after commit can therefore
leave an accepted record even when the call reports failure: query the original identity, never infer
non-acceptance from the error. SQLite's durability semantics are described in its
[WAL](https://sqlite.org/wal.html) and [synchronous](https://sqlite.org/pragma.html#pragma_synchronous)
documentation. Physical power-loss durability remains unverified.

## Run intent and recovery discovery

Before sending Start to a guardian, record a protocol-owned HostRunIntent with
record_run_intent(intent). It requires an existing Scope intent and saves the immutable ScopeId,
RunId, exact RunSpec and explicit host-disconnect policy. The whole guardian request must fit the
current wire frame limit before recording. Environment values are stored verbatim in this private
journal; native non-UTF-8 path/argument/environment values survive round trips.

The same RunId and intent return the original record; changing its Scope or parameters conflicts.
The call commits and syncs before returning. It does not create a Scope directory, launch a guardian,
dispatch Start, or promise automatic background execution. Host intent is distinct from guardian
acceptance, process facts and Node business success.

run_intent(run) queries one responsibility; run_intents() enumerates all host records in stable RunId
order, including completed or not-yet-dispatched attempts. This initial in-process enumeration uses
memory proportional to the retained journal; no retirement or pagination policy is implemented.
After recovery, the caller can obtain guardian_access(intent.scope), bind the new host incarnation,
and query the original Run through GuardianRuns. Losing an acknowledgement never requires a new ID.

Host formats before version 4 gain an empty Run table in the same transaction as the new host
binding. Version 3 may already have live guardians and Runs; migration neither reads guardian.sqlite
nor fabricates missing host intents. An empty host lookup is not proof that such a Run never executed.
Unknown formats, malformed indexed payloads and orphaned Run references fail before advancing authority.

## Durable stop and close intent

`request_run_stop(run)` records force-stop responsibility; `request_scope_close(scope)` permanently
seals host admission. Both commit before acknowledgement and remain effective after restart. Repeated
Start for an already recorded Run still returns its original intent, but new Runs and guardian launches
are rejected in a closing Scope. Close implicitly stops all its Runs without rewriting their original
specifications. Unknown identities are rejected. These records do not prove signal delivery or cleanup.

`run_stop_requested`, `scope_close_requested` and `scope_intents` expose the durable work to the host
coordinator. Existing versions gain empty control tables transactionally; v4 Run intents are preserved.
An orphaned control reference blocks recovery before advancing host authority.

## Verification and remaining work

`HostCoordinator` composes this journal with the guardian client. Its owner drives `tick()`;
accepted Start/Stop/Close requests then progress without the requesting connection. Each Scope has
at most one active exchange, with up to 32 independent Scope exchanges and round-robin scheduling.
The cap limits concurrent transport work, not admission. Failed exchanges back off from hundreds of
milliseconds to five seconds plus Scope-specific jitter, without exhausting cleanup responsibility.
Neither socket I/O nor bootstrap delivery holds the host journal while awaiting a peer.

Changed Run and Scope observations are persisted before queries expose them. `last_observed` is
historical evidence, separate from `coordination`; recovery starts as Pending and guardian loss
reports Unavailable without erasing earlier facts. No numeric PID or stored Running snapshot becomes
launch or signal authority. A close before any guardian attempt completes locally without exec;
otherwise the original guardian must answer. An unavailable guardian never becomes successful cleanup.

Dropping the coordinator stops its transport tasks, not guardians. Accepted intent remains in SQLite;
recovery rebinds and rediscovers the original instance. No same-Scope guardian relaunch is allowed.
Version 5 gains empty observation tables, preserving its stop/close intents. Earlier versions migrate
through the same exact schema checks; corrupt projections block recovery before authority advances.

`cargo test -p ora-process-runtime --test host_state` exercises concurrent creation, lock contention,
deduplication across restart, unchanged lock identity, lost caller state after external SIGKILL,
missing/foreign files, path limits, permissions, links, version/schema/identity corruption, epoch
exhaustion, conflict repair and the exact version-1 through version-5 upgrades, including unchanged legacy tokens and epochs on rejection. Run tests cover restart discovery,
immutable scope/parameter binding, failed commits, frame limits and corrupt indexed payloads. Crash fixtures override child HOME and cwd while using the same
explicit state path. Tests use a private temporary fixture under the test user's home; production
code does not derive a path from that environment variable.

Real-app bootstrap, refusal, launcher-kill and discovery evidence is documented under
[guardian bootstrap](../guardian.md), including durable host takeover and guardian-side Run acceptance.
Coordinator tests cover cancellation before dispatch, real side-effect deduplication after recovery,
durable facts after guardian loss and independent progress past an unavailable Scope. The host app is implemented;
Git/Node composition remains unfinished; Controller authorization is deferred. No ADR is marked implemented.
