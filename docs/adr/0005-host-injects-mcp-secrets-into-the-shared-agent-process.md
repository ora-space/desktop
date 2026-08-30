---
status: superseded by ADR-0008
---

# Host injects MCP Secrets into the shared Agent process

Ora Host maps MCP SecretRefs to stable environment variable names and injects their plaintext values only when spawning the trusted Agent CLI; Agent Plugins and Workspace configuration receive environment references instead. Because the minimum closed loop retains one shared Agent process, every Session on that process can observe the same plugin-global MCP Secrets, an accepted isolation trade-off that avoids plaintext IPC and Workspace credentials without introducing per-Workspace processes.

This design was superseded before implementation because the first release does not introduce a Secret Setting or SecretRef model.
