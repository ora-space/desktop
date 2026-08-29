# Agent Target domain

This module defines the Agent Target-shaped Effect persistence vocabulary used by the Expand
migration and typed repository APIs. An Agent Target is uniquely identified by Workspace identity
plus Agent Plugin identity. Status, reconcile requests, and conditions are owned by the target, not
by a physical Skill surface.

## Responsibilities

- Strong identities for Agent Targets, capability revisions, and target-owned conditions.
- Closed enums for target phase, condition impact, reconcile state, and wake reason.
- The `AgentTargetRepository` port for typed persistence without activating target-keyed workers.
- Complete record shapes suitable for whole-object repository round-trips.

## Non-responsibilities

- SQLite schema, backfill SQL, or repository transactions.
- Target-keyed worker claim loops, admission gates, MCP resolution, or OpenCode materialization.
- Compatibility views that hide the temporary coexistence of surface-keyed and target-keyed tables.
