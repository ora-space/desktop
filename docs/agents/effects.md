# Effects

English | [中文](effects.zh.md)

An **Effect** is something Ora must make true inside a Workspace for the agents that use it.
Installing Skills is the built-in Effect kind: installed Skills are materialized into every eligible
Workspace so agents can discover them. The generic machinery lives in `ora-effect`, the Skill
integration in `ora-effect-skill`, persistence in `ora-db`. The governing decisions are the
[Effect system foundation](https://github.com/ora-space/specs/blob/main/decisions/desktop/core/effect/0-effect-system-foundation.md)
and [Effect audit and business time](https://github.com/ora-space/specs/blob/main/decisions/desktop/core/effect/20260905-separate-audit-and-business-time.md)
ADRs.

MCP is deliberately not an Effect: it is Session runtime input with no Workspace files, see
[Session MCP](session-mcp.md).

## Model

- A Workspace is an `EffectScope` with one complete `DesiredState` generation. Desired State is a
  normalized set keyed by `DesiredEffectIdentity`; replacement uses generation compare-and-swap, and
  an exact normalized no-op does not advance the generation.
- An `EffectSource` is stable across publications; each publication creates an immutable
  `EffectRevision`. Retiring a source does not invalidate history or unfinished operations.
- Every runtime that consumes Effects (an agent plugin) declares a `Consumer` revision. Pairing a
  Consumer with a Scope creates an independent `EffectTarget`, the scheduling and readiness unit.
- Targets bind to `EffectResource`s, the independent observation, locking and mutation unit (for
  Skills, a directory in the Workspace). Targets and Resources are many-to-many: several Targets can
  contribute to one physical Resource.
- A `TargetProjection` is the complete deterministic view of one Target at one Consumer revision; a
  `ResourceProjection` merges every active or retiring Target bound to that Resource. A
  materialization contract flows unchanged from the Consumer's Resource template into the binding
  and requirement; contributors declaring different contracts block planning instead of one being
  chosen implicitly.
- `ManagedItem` is durable mutation authority. `ObservedItem` is only an adapter claim, and an item
  without exact ledger and marker evidence stays `PreservedItem` even if its bytes match Desired.
- Target watermarks satisfy `ready <= applied <= observed <= desired`. Resources have no readiness
  watermark, because only a Consumer can prove it can consume a complete projection.

Every Workspace has one Scope. Project and task Workspace repositories create the Scope and seed its
Desired State inside the Workspace transaction; publishing a new Skill source seeds existing Scopes.

## Convergence

Each Target owns one coalesced, level-triggered `ReconcileRequest`. A worker claim fences Target
status changes, and every bound Resource additionally takes its own monotonic claim before
observation can support status or readiness. Wake reasons are diagnostic only: after acquiring
authority the worker always reloads current Desired, declarations, statuses and ledgers.

1. Reload the Target declaration and claim every bound Resource in stable identity order.
2. Reload all mutable facts under those claims and project the Target and every shared contributor.
3. Observe and plan each Resource under its fence, never granting ownership from observation.
4. Persist the immutable Attempt, projections, prepared Operations and Artifact authority before
   any external side effect.
5. Persist Consumer coordination receipts and each monotonic Attempt and Operation phase between
   external calls.
6. Atomically finalize operation journals, ownership ledgers, statuses, readiness, Conditions and
   the request after exact adapter verification.

The Scope generation is the convergence epoch: projections are identified by
`(target_id, generation, consumer_revision_id)`, so anything that changes what a Target projects at a
generation opens a new epoch. Desired State replacement advances it, and so does a topology change:
retiring a Consumer (plugin uninstall) or replacing a Target declaration (plugin update) advances
every affected Scope in the same transaction, records a `topology_changed` audit event and wakes all
Targets. Observation, retry, Condition and runtime status changes never advance the epoch.

- An unchanged Consumer declaration touches nothing. New Consumers pair with existing Workspaces
  immediately, and every worker pass also pairs existing Consumers with Workspaces created later.
- A changed declaration retires the old Target and creates its replacement at the new epoch, so
  updates converge like uninstalls. A retiring Target keeps its bindings, contributes an empty
  Desired set to shared merges, and is deleted only after that empty projection converges.
- A Resource whose Workspace directory no longer exists observes as empty, so retirement can finish
  when nothing else desires content there. Mutation paths never recreate a Workspace root the user
  removed: if a surviving Target still targets it, both Targets keep failing until the directory
  returns or that intent is withdrawn.
- Republishing a previously retired Skill source restores its Desired intent in every active Scope,
  so reinstalling a plugin revision reconverges.

A newer wakeup preserves an active claim so the older pass can finish its exact projection. At
commit, Target status merges the latest Desired watermark with the proven progress: an older
completion cannot lower a watermark or report `Current` while newer work remains, and the newer
request stays pending for the next pass.

## Filesystem safety and recovery

Filesystem Resources are Workspace roots plus validated portable relative paths; the adapter
refuses links or ancestors that escape the Workspace. Each managed directory contains
`.ora-managed.json`. Mutation requires an exact match between the database ledger, the marker
identity, the native identity and the last applied fingerprint. A missing managed directory may be
rebuilt; unowned, marker-mismatched or drifted directories are never silently overwritten or
deleted.

Every mutation has a durable `Prepared → Applied → Finalized` journal with exact expected and
planned state. A transient failure before the journal exists enters a counted retry schedule. Once a
journal exists, an interrupted or ambiguous operation moves its Attempt, Target and Resource to
`RecoveryRequired` after the worker's lease expires, with blocking manual Conditions; a still-valid
worker is never quarantined. The scheduler excludes a recovering Target instead of guessing or
planning a second operation, and staging and backup Artifacts are kept until exact cleanup
authority succeeds. Nothing clears `RecoveryRequired` automatically; its one exit is the
controlled `recoverEffectTarget` command, which proves every quarantined operation against the
adapter before closing it. An operation whose external state equals its exact planned state
finalizes and keeps its ownership ledger row; one that still equals its exact expected baseline
closes as `FinalizedWithoutEffect` and grants nothing, because its mutation provably never became
durable. An operation that proves neither state keeps the Target quarantined — recovery never
replays a mutation, never grants unproven ownership, and never rewrites immutable projection
history. Closing the last journal clears the Target's manual Conditions, returns its request to
pending, and lets ordinary reconciliation converge against current Desired State; a retiring
Target additionally advances its Scope epoch so its empty retirement projection lands on a fresh
immutable identity. Recovering one Target never clears another Target's quarantine on a shared
Resource; the Resource keeps its evidence pointed at a still-quarantined operation.

Recovery revalidates the journal's persisted Workspace and Resource paths without creating
directories, and requires the resolved root to match the journal. Files, links, inaccessible paths
and redirected roots cannot prove a Missing state; a safely missing Resource can. The effects data
owner runs recovery and refreshes only selectors that observed that Target, so unrelated Workspace
and Agent readiness caches remain unchanged.

## Audit and business time

Effect repositories own row audit time: they sample an injected `TimestampSource` after acquiring
the write transaction and reuse that sample for the transaction. Each row preserves its existing
`updated_at` and any CHECK-required business lower bound, so a wall-clock correction can make audit
times equal but never regress them or confer generation or fencing authority.

The reconciler samples business time at preparation, after apply and after readiness; operations in
one Attempt share the preparation time. Recovery records an independent `detected_at` only while
an operation is `RecoveryRequired`. The closing transaction copies each operation's original
detection time, identity and closure outcome into the `recovery_finalized` audit event before clearing
its active `detected_at`; the event's `occurred_at` is the closing transaction's audit time, not the
detection time. Audit failure rolls back the closure as well. Worker
claims, recovery scans and retry scheduling take fresh samples, and a retry delay starts when the
failure is handled. Receipt `received_at` means local durable reception. Conditions keep their
identity and first observation and never regress their last observation. Status reads return audit
metadata with domain evidence from one snapshot. Local Skill `updated_at` remains the Skill's
revision key; an unchanged publication creates no revision.

## Interfaces

- `getEffectTargetStatus` exposes a Target's desired, observed, applied and ready generations,
  recovery identity and current Conditions to the frontend.
- `retryEffectTarget` wakes one Target without changing Desired State. The send gate surfaces
  Resource-owned blocking Conditions, such as a preserved Skill directory conflict, as a visible
  failure with the conflicting names; after the user removes the external cause, this command
  retries the Target and refreshes only its readiness caches.
- `recoverEffectTarget` is the controlled exit from `RecoveryRequired`; the chat send gate offers
  it while readiness is blocked by a recovery quarantine, and a refusal keeps the quarantine.
- Plugin registration exposes `effectResources`. Agent Consumers implement `effect/coordinate`,
  `effect/reactivate` and `effect/verify_ready`; those payloads stay behind the `ConsumerAdapter`
  boundary and add no agent-specific phases to the generic machinery.
- Package adapters compute immutable fingerprints before publication; SQLite stores the digest and
  fingerprint but never reads package directories through the Resource adapter.
- Skill Effect mutation and MCP refresh share the Agent Session Barrier, see
  [Session MCP](session-mcp.md#live-refresh).

Schema: migration `0006` (Scopes, Sources and Revisions, Desired State, Consumers and Targets,
Resources and bindings, projections, ownership, statuses, Conditions, requests and claims,
Attempts, Operations and Artifacts, receipts, audit events), `0010` (independent recovery
detection time) and `0019` (the `finalized_without_effect` closure phase, added by rebuilding the
operation journal through staging copies). See [Persistence](../core/persistence.md).

## Windows contention and diagnostics

A reader that opens a file without delete sharing can prevent Windows from renaming its parent
Skill directory, returning access denied (5) or a sharing/lock violation (32/33). Directory swaps
retry these Windows errors at 50 ms intervals for up to one second per rename. Other errors and
other platforms fail immediately. Persistent locks and ACL restrictions still fail safely: Ora
neither terminates the reader nor changes permissions, deletes the destination, or copies over it.
Stop the activity holding the Skill files before trying explicit recovery again.

Recovery success means journal closure, not readiness: `finalized_operations` and
`closed_without_effect_operations` distinguish exact planned from exact baseline proof. A new
reconcile may still fail while the original access restriction remains. Diagnose that sequence using:

- `effect_recovery`: Target identity, start, and closure counts; subsequent convergence is still required.
- `effect_reconcile`: a per-pass `trace_id` links the Target to adapter events and the final outcome.
- `skill_materialization`: immutable operation/attempt/Resource identities, generation, mutation,
  stage, outcome and duration. `stage_package`, `verify_planned` and `verify_expected` distinguish
  copy/validation from state proof. `backup_previous` and `install_staging` record both rename paths,
  attempts on success, and the OS error on failure. IO error chains name the exact action.
- Rollback and artifact cleanup failures are logged rather than silently discarded.

Logs never include Skill file contents, ownership marker bytes or credentials. Filesystem paths
in local diagnostic logs may identify the user's workspace; redact them before sharing publicly.
