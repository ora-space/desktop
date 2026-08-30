---
status: accepted
---

# Bind sensitive String Settings through the Agent environment

The first release keeps Tavily `apiKey` as an ordinary String Setting stored in Ora's permission-restricted plugin `store.json`; it does not introduce Secret or SecretRef types. A Setting reference used in Tavily's HTTP Authorization header becomes a deterministic, collision-resistant environment name derived from the canonical Plugin ID, Setting ID, and binding position. The Host resolves prefix, stored value, and suffix into the final environment value and sends the complete binding set to the trusted Agent Plugin for the next Effect activation.

The OpenCode renderer writes only `{env:VARIABLE}` references. It JSON-encodes values and escapes braces before OpenCode's `{file:...}` substitution pass, rejects control characters that are illegal in HTTP headers, and exact-redacts both actual and escaped known values from Agent and child-process diagnostics. Plaintext may transit trusted IPC and memory before entering the OpenCode child-process environment, but it must never enter Effect state, the Workspace file, the ownership marker, logs, or errors. A future Secret model may replace this transient path without changing the native environment references.
