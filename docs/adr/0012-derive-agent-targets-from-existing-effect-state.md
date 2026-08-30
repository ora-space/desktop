---
status: superseded by ADR-0015
---

# Derive Agent Targets from existing Effect state

The derived Agent Target projection and its aggregate generations are superseded for the first release by ADR-0015. P1 keeps the existing Surface and Surface Consumer as the durable scheduling/readiness units and groups only the current worker claim batch in memory for shared activation.

The first release does not add `effect_agent_targets`, target-to-surface, target-status, or target-request tables. An Agent Target is the domain projection identified by `(workspace_id, agent_plugin_id)`: its members come from `effect_surfaces` joined through `effect_surface_consumers`, its desired generation comes from `workspace_effects`, its applied state comes from the member surface statuses, and its ready state and conditions come from the corresponding consumer statuses. Aggregate applied and ready generations are the minimum across required members, and the strictest member condition determines the projected phase.

The worker continues to claim durable per-surface reconcile requests, groups the claims by derived Agent Target, prepares existing per-surface operations under a shared batch identity, coordinates the target once, and later batches one restart across the successful targets of a shared Agent. Requests remain owed until activation and consumer readiness are recorded. Startup recovery is extended to rearm work when a consumer ready generation lags its surface desired generation. This retains the current ownership, journal, lease, and surface model without creating a second source of truth; a persisted AgentTarget aggregate is deferred until actual multi-process or independently scheduled target lifecycle requirements justify it.
