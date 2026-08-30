---
status: superseded by ADR-0015
---

# Keep migrations pure and gate the release with redacted tests

ADR-0015 keeps the pure-migration rule but supersedes the mandatory release-gated live smoke and the P3 test matrix. P1 provides hermetic tests plus an explicit opt-in Tavily smoke using an externally supplied key; making that smoke a release gate is a separate release-policy decision.

Database migrations change only schema and persisted database data. They do not traverse Workspaces, invoke plugins, access the network, or write Agent configuration files. After startup, source synchronization and the existing Effect convergence path seed current Ready MCP definitions into existing Workspace generations, derive their Agent Targets, and perform ordinary journaled materialization.

Contract v2 and Effect boundaries return stable typed codes such as `NeedsConfiguration`, `CapabilityMismatch`, `UnsupportedWorkspace`, `TrackedConfig`, `OwnershipConflict`, `DriftConflict`, `ObservedChanged`, `GitExcludeFailed`, and `RecoveryRequired`. User messages contain only approved paths, identities, and digests; raw parser, Git, child-process, and plugin errors are not surfaced when they may contain Setting values.

Hermetic tests in `desktop-mcp` cover install identity validation, pure MCP resolution, configuration revisions, derived-target grouping, crash recovery, admission gates, shared restart, live/warm invalidation, and a fake Contract v2 Agent tool invocation. `opencode-agent` tests cover JSON/JSONC preservation, sidecar ownership, Plan CAS, environment references, and idempotent recovery. A separate release-gated smoke test receives `TAVILY_API_KEY` from release secrets and verifies the public marketplace install, configuration, conversation, MCP tool-call event, and successful result without emitting the key; ordinary pull requests and local tests use fakes and require no network credential.
