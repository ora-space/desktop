# ora-node

[中文](README.md)

The Rust Node requires mutual TLS, a fixed platform scope, and root management separated from workload UID 1000 for cloud network operation. Protected credentials and local journals stay outside code. Durable acceptance and first directory/Git mutation check eligibility separately; recovery closes old incarnations. Accepted work settles within its original scope. Plaintext production networking and global personal Git fallback are refused. The loopback transport example is test-only, never shipped, and requires injected test bindings.

## Verification boundary

Declared model execution validates its management-owned archive receipt, declaration, target, protocol and Settings before acquiring authority over dedicated mTLS. Receipts live under `plugins/.node-receipts`; pre-upgrade packages without proof require a verified re-download for declared execution. Historical OpenCode bindings retain their explicit compatibility route. Only the live startup RPC carries the temporary token, and exact application acknowledgement fences the first prompt. Controller inputs, ledgers and history never contain the token. Plugins and children start with isolated HOME/state and an empty inherited environment. Node owns renewal, revocation and cleanup.

Module tests cover their real protocol, recovery, Git or TLS boundary. Cloud doubles and the loopback fixture do not prove production multiplayer authorization. Use cluster Compose for acceptance. The third_party/cloud gitlink pins the contract; generated output is never edited by hand. See [runtime control](../../docs/controller/runtime-control.md) for dependencies and missing evidence.
