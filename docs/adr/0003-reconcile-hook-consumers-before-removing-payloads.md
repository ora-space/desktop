---
status: proposed
---

# Reconcile Hook consumers before removing payloads

Once Agent Plugins can consume Hook Plugins, disabling, uninstalling, or upgrading a Hook Plugin must first move affected consumers to the new desired state and wait for the required idle-and-restart barrier before removing the old executable payload. This prevents the durable plugin state from claiming success while an agent still uses the old Hook, or from invalidating a path held by a running agent.

This invariant is intentionally not implemented by the installation-only RTK milestone because that milestone has no Agent Plugin consumption interface.
