# Unsupported MCPs do not block Agent Targets

When an Agent plugin cannot materialize one Ready MCP, reconciliation records a target-specific `UnsupportedByAgent` issue, applies the remaining supported MCPs, and advances the Agent Target to Ready with Issues. Because Ready MCPs propagate to every Workspace automatically and support varies by Agent plugin, failing the entire target would allow one globally installed MCP to disable otherwise usable Agents; capability or Desired State changes trigger reevaluation of the skipped item.
