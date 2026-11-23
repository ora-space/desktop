# ora-node

[中文](README.md)

The Rust Node requires mutual TLS, a fixed platform scope, and root management separated from workload UID 1000 for cloud network operation. Protected credentials and local journals stay outside code. Durable acceptance and first directory/Git mutation check eligibility separately; recovery closes old incarnations. Accepted work settles within its original scope. Plaintext production networking and global personal Git fallback are refused. The loopback transport example is test-only, never shipped, and requires injected test bindings.

With Agents enabled, one checkout hosts at most 8 unfinished sessions. Prior-Revision restore and unknown stop evidence still require exclusivity. `checkout_busy` and `checkout_session_limit` end only the incoming session without closing the control connection. The handshake advertises `shared_checkout_sessions`.

With `process.workload_uid` set, the host must set the sysctl `fs.protected_hardlinks = 1` (the Node refuses to start otherwise and reads it again before every checkout hand-over; it is not namespaced, so containers inherit the host's value) and give the Node a soft `RLIMIT_NOFILE` of at least 38,000 (65,536 recommended): each hand-over walk holds up to 4,099 descriptors, the Node runs at most 9 walks at once however many checkouts are deep, and a checkout nested deeper than 4096 directories is refused as `checkout_too_deep`. See [handing a shared checkout to the workload user](../../docs/node/agent-sessions.md#handing-a-shared-checkout-to-the-workload-user).

## Verification boundary

Module tests cover their real protocol, recovery, Git or TLS boundary. Cloud doubles and the loopback fixture do not prove production multiplayer authorization. Use cluster Compose for acceptance. third_party/cloud pins contract fe33eb49633028f3fd466de966d3a5754d126557; generated output is never edited by hand. See [runtime control](../../docs/controller/runtime-control.md) for dependencies and missing evidence.

## Session MCP

`session_mcp` advertises explicit `AgentSessionSpec.mcp` bindings (plugin id, exact version, scalar
settings). Omitted/empty means none; no automatic discovery occurs. Settings stay in memory and
the successful ACP `session/new`, never the ledger or history. See [Session MCP](../../docs/agents/session-mcp.md#node-sessions)
for limits, capability gates, failure codes and package isolation.

`session_host_mcp_servers` advertises `AgentSessionSpec.host_mcp_servers`: Controller-provided HTTP
servers attached after the plugin servers on every ACP setup, with URLs and headers kept in memory
and only their names in the ledger. See [Host MCP servers](../../docs/agents/session-mcp.md#host-mcp-servers).
