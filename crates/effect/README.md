# ora-effect

`ora-effect` owns Ora's workspace-scoped declarative Skill State and the machinery that safely
projects it onto consumer-declared filesystem surfaces.

## Responsibilities

- Strong identities for selections, exact source states, managed ownership, surfaces, operations,
  and generations.
- A pure planner that separates desired, managed, observed, and preserved state and never grants
  ownership from disk contents alone.
- Consumer descriptor merging, structured conditions, retry policy, and per-consumer readiness.
- Safe filesystem scanning and journaled staging/swap/delete operations with deterministic crash
  recovery decisions.
- Repository, source, consumer-coordination, and clock ports used by a statically dispatched
  reconciler.

## Boundaries and invariants

The crate does not depend on SQLite, Tauri, a concrete Agent runtime, or hard-coded consumer paths.
Only a matching database ledger and `.ora-managed.json` marker authorize mutation. A matching name,
digest, source package, or orphan marker is never ownership evidence. Directory fingerprints cover
all materialized content except the ownership marker.

Reconciliation plans a complete surface before mutation, serializes one physical surface at a
time through its repository request, and treats watcher payloads only as wakeups. Different
surfaces may be driven concurrently by the host.
