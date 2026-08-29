# plugin

`PluginApi` is the backend composition of plugin discovery, lifecycle, configuration, and the
process-local Agent Effect declaration snapshot. Agent plugins publish Skill surfaces and optional
MCP configuration into one map; uninstall and Skill-only Expand tests write through the same API.

## Responsibilities

- Own plugin process lifecycle, marketplace install/update, and configuration service wiring.
- Store one Agent Effect declaration per running Agent plugin: Skill surfaces, negotiated MCP
  capability, and Agent Capability Revision.
- Persist merged Skill surfaces to every local Workspace and wake Effect reconcile after commit.
- Expose `agent_effect_surface_declarations` as the single snapshot Skill-surface convergence reads.

## Non-responsibilities

- Interpreting ACP or rendering Agent-native MCP documents.
- Target-keyed MCP materialization, admission gating, or condition persistence (`#489`).
- Discovering MCP packages or publishing Source revisions (`#486`).

## Invariants

- Skill surfaces and MCP capability share one per-plugin map. Convergence must not consult a
  second registry.
- An absent declaration (no Skill surfaces, no MCP negotiation, no bound revision) removes the
  plugin from the snapshot so uninstall cannot leave a ghost consumer.
- Live attach always binds a capability revision from the exact installed plugin version and the
  canonical capability digest. Skill-only Expand writes use `AgentPluginEffectDeclaration::skill_only`.
