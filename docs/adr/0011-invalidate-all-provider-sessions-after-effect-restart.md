---
status: accepted
---

# Invalidate all provider Sessions after an Effect restart

One Agent connection serves every Workspace, and the plugin's existing `effect/restart` replaces the underlying Agent process without necessarily replacing the Ora plugin transport. Every provider Session identifier from before that boundary is therefore dead, including warm Sessions that have no live runtime actor. After restart, Ora uses the existing replacement command so matching live actors detach and become Stopped, and it cools all matching warm-pool provider bindings while preserving their Ora identifiers and desired Session configuration for later rebuilding.

P1 does not add a centralized admission gate or a new persisted replacement epoch. The Settings surface and end-to-end flow wait for the affected MCP surfaces to become Ready before creating a new conversation; broader all-path stale-generation prevention remains deferred by ADR-0015.
