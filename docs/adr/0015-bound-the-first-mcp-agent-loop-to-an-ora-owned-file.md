---
status: accepted
---

# Bound the first MCP-to-Agent loop to an Ora-owned Workspace file

The first release uses the P1 profile to preserve the original physical-file acceptance test without turning it into a general Agent configuration platform. A runtime `agent_mcp_v1` declaration lets a compatible Agent Plugin render one complete native file from Ora's normalized HTTP MCP set; the Host validates and atomically applies that file through an independent MCP Effect surface. Existing local Workspaces are reconciled eagerly, and the worker reuses the Agent's existing wait/restart coordination, grouping only its current claim batch so each shared Agent is activated at most once in that batch.

For OpenCode, Ora manages only a newly created `.opencode/opencode.jsonc`. If any target file already exists without matching database ownership and the inline file-header marker, materialization fails without modifying it. P1 does not merge user JSON/JSONC and does not create an independent sidecar. MCP keys are collision-resistant derivations of canonical Plugin IDs; Ora scans Workspace-visible OpenCode layers and fails on a known collision, while collisions in unobservable global or managed layers remain a documented first-release limitation.

Configuration completeness and application readiness remain different facts. A complete MCP with no registered MCP-capable Agent is `WaitingForAgent`; it becomes `Ready` only after the Ora-owned file, ownership ledger, and Agent activation have converged for all registered local surfaces. The closed-loop conversation and smoke test start only after Ready. P1 does not create a universal admission gate for every warm, load, workflow, or prompt path.

## Consequences

- Skill and MCP desired sets remain typed and independent; the existing Workspace generation is only a coarse convergence epoch, not a promise of atomic combined readiness.
- Surface and Surface Consumer rows remain the durable units. No persisted or derived Agent Target aggregate is introduced.
- A worker claim batch may cause one activation per shared Agent; later batches, retries, or newer revisions may cause another activation. There is no durable global exactly-once cohort.
- The first runtime profile supports Tavily's remote HTTP MCP only. stdio process lifecycle, additional Agents, remote Workspaces, user-config merge, independent sidecars, static Contract v2 capability matrices, full retirement orchestration, Workspace exclusions, and all-path stale-generation gates are deferred.
- Tavily's API key remains a plaintext String in Ora's plugin `store.json`, but only an environment reference is persisted in the Workspace file. Plaintext must be safely encoded for OpenCode substitution and redacted from every diagnostic path.
- Live validation is an explicit opt-in smoke using an externally supplied `TAVILY_API_KEY`; it is not a mandatory ordinary-PR or release gate in this phase.
