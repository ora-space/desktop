---
status: proposed
---

# Bind Agent instances to Configuration Snapshots

Agent consumption is deferred from the first Plugin Configuration delivery. A future integration should bind each Agent instance to an immutable Configuration Snapshot so saving a new Configuration Revision cannot silently mutate or restart running instances.
