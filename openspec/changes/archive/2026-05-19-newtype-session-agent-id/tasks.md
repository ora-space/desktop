## 1. Introduce the typed session agent identifier

- [x] 1.1 Add `ora_domain::AgentId` as the session-owned transparent newtype, including the canonical terminal constant and any constructors or accessors needed by current session callsites.
- [x] 1.2 Update `ora_domain::Session` and its tests to use `AgentId` for `agent_id`, keeping serde-visible behavior string-compatible.

## 2. Propagate the new type through persistence and callsites

- [x] 2.1 Update the SQLite session repository to write `AgentId` to the existing `sessions.agent_id` text column and to rebuild `Session` values with `AgentId` on reads.
- [x] 2.2 Update application, contract-mapping, and repository test fixtures that construct `Session` values so they use `AgentId` explicitly while preserving current transport payloads.

## 3. Verify the change end to end

- [x] 3.1 Refresh any affected docs under `docs/` if they describe session agent identity as a raw string API detail.
- [x] 3.2 Run `cargo fmt --all` and `task test`, then address any failures caused by the typed session agent identifier rollout.
