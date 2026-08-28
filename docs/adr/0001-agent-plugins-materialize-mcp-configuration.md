# Agent plugins materialize MCP configuration

Ora resolves installed MCP packages into target-independent `ResolvedMcp` values, while each Agent plugin renders and reconciles those values into its target Agent's native configuration. MCP delivery reuses the Workspace-scoped, durable convergence and coordination semantics proven by Skill Effects, but it does not reuse the Skill filesystem adapter or move target-specific configuration schemas into the Ora host.
