## Why

`Session.agent_id` currently uses a raw `String`, which makes a core domain identity easy to mix up with unrelated text and leaves known agent values such as `terminal` outside the type system. Tightening this field now lets the domain model express intent directly and gives downstream adapters one canonical shape for session agent identifiers before more agent-specific behavior accumulates.

## What Changes

- Replace the `ora-domain` session `agent_id` field with a dedicated `AgentId` newtype that serializes transparently as a string.
- Add domain-owned known agent constants, starting with the terminal agent identity.
- Update session constructors, repository mappings, and related tests to use the typed identifier instead of raw strings.
- Keep persisted SQLite values and transport payloads string-compatible while moving domain and repository code onto the stronger type.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `domain-models`: change session modeling so agent identifiers are represented as a dedicated typed value instead of a raw string.
- `database-repositories`: change SQLite session mapping so persisted `agent_id` text is encoded and decoded through the new domain `AgentId` type.

## Impact

- Affected code: `crates/domain`, `crates/db`, and any application or contract tests that construct `Session` values directly.
- Affected APIs: the internal Rust domain API for `Session::agent_id` and any constructors or helpers that currently accept raw session agent strings.
- Dependencies: no new external dependencies are expected.
- Systems: SQLite storage and serialized payloads remain string-compatible, but internal callers must move to the typed domain identifier.
