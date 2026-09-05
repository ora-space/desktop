# Effect Skill State

Ora models Skill installation as one kind of generic Effect. A Workspace is an `EffectScope` with
one complete `DesiredState` generation. Every runtime that consumes Effects declares a stable
`Consumer` revision; pairing that Consumer with a Scope creates an independent `EffectTarget`.
Targets bind to independently observable and mutable `EffectResource` values, and several Targets
may contribute to the same physical Resource.

The current architectural decision and its rationale are recorded in
[`0-effect-system-foundation.md`](../specs/decisions/desktop/core/effect/0-effect-system-foundation.md);
the implementation and this state description are updated together when that decision changes.

## State and identity

- `EffectSource` is stable across publications. Each publication creates an immutable
  `EffectRevision`; retiring a source does not invalidate history or unfinished operations.
- `DesiredState` is a normalized, complete set keyed by `DesiredEffectIdentity`. Replacement uses
  generation compare-and-swap, and an exact normalized no-op does not advance generation.
- A `TargetProjection` is the complete deterministic view for one Target and exact Consumer
  Revision. A `ResourceProjection` merges the requirements of every active or retiring Target
  bound to that Resource.
- A materialization contract flows unchanged from the Consumer Resource template into its Target
  binding and projection requirement. Contributors declaring different contracts for one shared
  Resource block planning instead of selecting one implicitly.
- `ManagedItem` is durable mutation authority. `ObservedItem` is only an adapter claim, and an item
  without exact ledger and marker evidence remains `PreservedItem` even if its bytes match Desired.
- Target watermarks satisfy `ready <= applied <= observed <= desired`. Resource status has no
  readiness watermark because only a Consumer can prove that it can consume a complete Target
  projection.

## Scheduling and convergence

Each Target owns one coalesced, level-triggered `ReconcileRequest`. A worker claim fences Target
status changes; every bound Resource additionally uses an independently monotonic Resource claim
before observation can support status or readiness. Wake reasons are diagnostic only, so a worker
always reloads current Desired, declarations, statuses, and ledgers after acquiring authority.

The reconciler follows this evidence chain:

1. Reload the current Target declaration and claim every bound Resource in stable identity order.
2. Reload all mutable facts under those claims, then project the complete Target and every shared
   Resource contributor.
3. Observe and plan each Resource under its fence without granting ownership from observation.
4. Persist the immutable Attempt, projections, Prepared Operations, and Artifact authority before
   any external side effect.
5. Persist Consumer coordination receipts and every monotonic Attempt/Operation phase between
   external calls.
6. Atomically finalize operation journals, ownership ledgers, statuses, readiness, Conditions, and
   the Target request after exact adapter verification.

An unchanged Consumer declaration does not touch Target status or requests. New Consumers are
paired with existing Workspaces immediately, while every worker pass converges existing Consumer
declarations into Workspaces created later.

## Audit and business time

Effect repositories own row audit time. They sample an injected `TimestampSource` after acquiring
the write transaction and reuse that sample throughout the transaction. Each row independently
preserves its existing `updated_at` and any CHECK-required business timestamp lower bound; new rows
use the write sample for `created_at` and `updated_at`. A wall-clock correction may make audit times
equal but cannot regress a row or confer generation/fencing authority.

Local Skill `updated_at` remains its revision key and package-journal version. Publishing an old
Skill into a newer Scope or Target uses the Effect write clock for audit fields, without changing
the Skill version. An unchanged publication does not create a revision or advance generation.

The reconciler samples business time at preparation, after apply, and after readiness. Operations
in one Attempt share preparation time; later phase times preserve their own preceding phase lower
bound. Recovery stores an independent `detected_at`. Worker claims, recovery scans, and retry
scheduling take fresh samples; retry delay starts when failure is handled. Receipt `received_at`
means local durable reception. Conditions preserve their identity and first observation and never
regress their last observation. Target status reads return audit metadata alongside domain evidence
and Conditions from one database snapshot.

A newer wakeup preserves an active claim so the older reconcile can finish its exact projection
and any prepared filesystem operations. At commit, Target status merges the database's latest
Desired watermark with the proven progress; an older completion cannot lower any watermark or
report Current while newer Desired work remains. The newer request stays Pending for the next
pass. Status versions preserve intervening updates and advance for changed status, while identical
status replay does not manufacture a new version. Audit-time protection alone does not provide
this business-state merge.

The decision is recorded in
[Effect audit and business time](../specs/decisions/desktop/core/effect/20260905-separate-audit-and-business-time.md).

## Filesystem safety and recovery

Filesystem Resources use Workspace roots plus validated portable relative paths. Path construction
uses typed path APIs, and the adapter refuses links or ancestors that escape the Workspace.

Each managed directory contains `.ora-managed.json`. Mutation requires an exact match between the
database ledger, marker identity, native identity, and last applied fingerprint. Missing managed
directories may be rebuilt; unowned, marker-mismatched, or drifted directories are never silently
overwritten or deleted.

Every mutation has a durable `Prepared -> Applied -> Finalized` journal and exact expected/planned
state. A transient failure before journal preparation enters a counted retry schedule. Once a
journal exists, any interrupted or ambiguous operation, its Attempt, Target, and Resource enter
`RecoveryRequired` after its worker lease expires, with blocking manual Conditions; a still-valid
worker is never quarantined. The scheduler excludes the recovering Target instead of guessing or
planning a second operation. Operation-owned staging and backup Artifacts remain retained until
exact cleanup authority succeeds.

## Persistence and protocol boundaries

SQLite migration `0006` stores Scopes, Sources/Revisions, complete Desired State, Consumer
Revisions, Targets, Resources/bindings, digest-addressed projections, ownership ledgers, independent
statuses, Conditions, requests/claims, Attempts, Operations/Artifacts, readiness/coordination
receipts, and append-only audit events.

Catalog/package adapters compute immutable package fingerprints before publication. SQLite stores
the validated digest and fingerprint but does not read package directories through the Effect
Resource adapter.

Plugin registration exposes `effectResources`. Agent Consumers implement `effect/coordinate`,
`effect/reactivate`, and `effect/verify_ready`; those versioned payloads remain behind the
`ConsumerAdapter` boundary and do not add Agent-specific phases to Effect Core.

Migration `0010` separates recovery detection from row audit time and removes the Workspace insert
trigger that inherited historical catalog timestamps. Project and task Workspace repositories now
create the Scope, seed Desired, and record wakeups within the Workspace transaction. Existing
Effect identities, ownership, journals, requests, and historical audit values are retained.
Rollback writes recovery detection back to the legacy column and restores the trigger before an
older application runs; the next upgrade reconstructs detection evidence again.
