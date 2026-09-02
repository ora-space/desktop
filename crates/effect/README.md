# ora-effect

`ora-effect` owns Workspace-scoped Desired Effect convergence and the closed, strongly typed wire
model persisted by the built-in implementation. Planning logic and concrete Resource adapters live
in integration crates such as `ora-effect-skill`.

## Responsibilities

- Strong identities and immutable revisions for Scopes, Sources, Desired Effects, Consumers,
  Targets, Resources, projections, ownership, Attempts, Operations, and Artifacts.
- One statically dispatched `EffectPlanner` interface for complete Target projection and
  shared-Resource merge planning with structured Conditions.
- Independent Target readiness and Resource materialization watermarks.
- Level-triggered requests, fenced Target/Resource claims, retry schedules, and journal-backed
  recovery states.
- Static-dispatch repository, planner, Consumer adapter, and Resource adapter seams. Resource
  operation preparation is part of the Resource adapter interface so callers cannot combine
  mismatched preparation and execution implementations.

## Boundaries and invariants

`EffectTarget` is the Consumer scheduling/readiness boundary; `EffectResource` is the independent
observation, locking, mutation, ownership, and recovery boundary. They are many-to-many and must not
share identity or status.

Desired, Managed, Observed, and Preserved state remain distinct. Only a matching ledger or durable
Operation/Artifact authorizes mutation. Target claims never authorize shared Resource writes, and
the reconciler reloads and replans after Resource claims close the race with other Targets.

The crate does not depend on SQLite, Tauri, a concrete Agent runtime, or Skill package parsing.
Built-in Consumer- and Resource-specific data is represented by closed versioned payload enums in
this crate, but interpreted only by its integration crate. MCP is Session Runtime Input, not an
Effect kind: adding a built-in Effect kind therefore extends both the wire enum and its
integration planner; it does not add kind-specific branches to the generic status, claim,
watermark, or recovery state machines.
