---
status: superseded by ADR-0015
---

# Agent Contract v2 declares MCP capability

The static and runtime Contract v2 matrix is superseded for the first release by ADR-0015. P1 discovers the optional `agent_mcp_v1` runtime surface through the existing Agent registration method set and uses one constrained complete-file renderer.

Agent Contract v2 requires an Agent Plugin to declare the MCP inputs it can safely express and to implement complete-set MCP configuration. The installed Agent package declares contract version 2 in its contribution metadata, and runtime registration repeats the version, methods, supported transports, and value-binding forms; Ora rejects a mismatch. The first OpenCode profile supports stdio and HTTP transports plus literal and `environment_reference_v1` bindings. Its `agent/configureMcp` method carries `{ workspaceId, cwd, generation, mcps }` together with a discriminated `plan`, `apply`, or `observe` action. Planning returns artifact locators and previous/planned fingerprints without sensitive values; Ora persists Prepared state before Apply, and Observe supports deterministic recovery and finalization. A completed operation returns the applied generation, whole-config fingerprint, and each managed entry's identity, target key, and fingerprint.

Contract v1 Agents remain usable and are excluded from automatic MCP materialization. If a v2 Agent Target cannot express any Ready MCP in its complete desired set, Ora reports a capability mismatch and blocks only that target from creating or admitting Sessions; it never silently omits the incompatible MCP. Release order is Host-first: a Contract v2-capable Ora release ships before the immutable `ora-space.opencode` 0.4.0 package, whose engine constraint names that minimum Host version; only then is the marketplace index advanced. OpenCode 0.3.0 remains unchanged, and the existing Tavily MCP 0.1.0 release is reused.

The first release deliberately does not make Agent package activation transactional. Ora's current updater commits a verified newer package and retires older Agent version directories before the new runtime registration/start is proven. If OpenCode 0.4.0 then fails Contract or runtime startup, the Agent remains unavailable for manual recovery rather than automatically falling back to 0.3.0. This is an accepted scope and availability tradeoff, not the policy for exact MCP package versions retained by Managed MCP references.
