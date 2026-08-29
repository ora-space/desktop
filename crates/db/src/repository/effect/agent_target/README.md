# Agent Target SQLite persistence

This module implements `ora_effect::AgentTargetRepository` on `SqliteEffectRepository`. It owns
typed reads and writes for Agent Target rows, status, target-owned conditions, and target-keyed
reconcile requests introduced by migration `0007`.

## Responsibilities

- Upsert and load Agent Targets uniquely keyed by Workspace identity plus Agent Plugin identity.
- Persist status generations (`desired` / `observed` / `applied` / `ready`), phase, status version,
  and Blocking/NonBlocking conditions as whole objects.
- Upsert target reconcile requests with max-generation and earliest-due coalescing.
- Keep these APIs independent of the surface-keyed Skill claim loop.
- `encode` owns enum encodings and reuses Skill generation integer conversion. `rows` maps identity
  and request rows. `conditions` loads and replaces target-owned conditions.

## Non-responsibilities

- Claiming or completing target reconcile requests in production workers. Lease columns exist so a
  later worker ticket can claim them without another schema migration.
- Dual-writing surface and target requests, compatibility views, or MCP materialization.
- Mutating or retiring the existing `effect_reconcile_requests` / `effect_conditions` tables.
