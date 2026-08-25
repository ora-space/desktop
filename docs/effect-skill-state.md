# Effect Skill State

Effect stores one complete declarative Skill selection set per Workspace and projects it onto each
consumer-declared physical surface. The first implementation deliberately has no product worker or
real Agent runtime integration; tests and later composition code drive reconciliation explicitly.

## State and identity

- Desired is a normalized complete set keyed by source kind, namespace, and case-insensitive Skill
  name. Replacement uses generation compare-and-swap and an exact normalized no-op does not advance
  generation.
- Managed is the database ownership ledger. A random `ManagedIdentity` remains stable across content
  updates and ends only after safe Desired removal or surface retirement.
- Observed and Preserved come from each live filesystem scan and are never persisted. An existing
  directory without matching ledger plus marker proof remains Preserved even when its bytes match a
  catalog source.
- Source version locates an upstream package revision; the SHA-256 `SKILL.md` digest verifies the
  exact declaration bytes. The applied directory fingerprint covers paths, entry types, file bytes,
  and executable intent while excluding the ownership marker.

## Filesystem safety and recovery

Consumer descriptors with the same normalized path and materialization format share one surface.
Path changes retire the old persisted surface rather than changing its ledger identity. Surface
creation verifies every existing Workspace-relative ancestor and rejects links.

Each managed directory contains `.ora-managed.json`. Mutation requires that marker identity,
Workspace, and surface to match the database ledger and that the live directory fingerprint match
the last applied value. Missing managed directories may be rebuilt with the same ownership;
unowned, marker-mismatched, or drifted directories are never overwritten or deleted.

Per-resource operations use durable `Prepared → Applied → Finalized` records. Staging and backup
paths are operation-specific and stored in the journal. Recovery retries when disk matches the
previous fingerprint, finalizes when it matches the planned fingerprint, and enters manual
`RecoveryRequired` for every other state. A newer Desired generation cannot bypass unfinished work.

## Persistence and source propagation

SQLite migration `0006` stores source revisions, Desired rows, surfaces, ledgers, status, consumer
readiness, operations, and coalesced reconcile/propagation requests. Desired replacement and source
delete/rename protection use immediate write transactions. Local Skill update commits its catalog
row, exact digest/source revision, and propagation request together. The explicitly driven
propagator rereads the latest source and advances only Workspaces that still reference its stable
selection, so V1→V2→V3 can materialize V3 directly.

See [the implementation plan](../specs/changes/effect/plan.md) for the first-version acceptance
matrix and deferred real-Agent integration work.
