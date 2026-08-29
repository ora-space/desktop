# mcp_configuration

This module owns the Host wire adapter for the optional MCP Configuration Capability:
capability/handler pairing, full-snapshot request construction, transport exclusion,
strict receipt validation, and Agent Capability Revision binding.

## Responsibilities

- Negotiate `mcpConfiguration` against `agent/configureWorkspace` without invalidating the
  baseline Agent conversation contract.
- Build a complete snapshot that carries operation identity, Agent Target identity, Workspace
  root, generation, and supported Resolved MCP values only.
- Exclude unsupported transports before the plugin call and represent them as target-level
  NonBlocking `UnsupportedByAgent` conditions.
- Reject receipts that do not exactly cover the supported Desired set.
- Bind Agent Capability Revision to the exact plugin version and canonical capability digest.
- Keep header values, environment values, document bytes, and API keys out of diagnostics.

## Non-responsibilities

- Effect worker cutover, admission gating, and persistence of conditions (`#489`).
- OpenCode-native document rendering (`#488`).
- MCP Source publication and Desired Set membership (`#486`).
- Publishing the plugin SDK (`#495`).

## Invariants

- Missing, malformed, duplicate, or unpaired capability disables MCP materialization only.
- Snapshot JSON never includes raw manifests, configuration stores, database paths, or
  unrelated plugin paths.
- JSON-RPC timeout errors and Host traces name the method, never the payload.
