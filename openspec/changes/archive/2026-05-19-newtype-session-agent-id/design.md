## Context

`ora-domain::Session` currently stores `agent_id` as a plain `String`, and the SQLite repository persists that field as raw text. The codebase already treats session agent identity as a meaningful domain concept, but there is no dedicated type to express that meaning or to centralize known agent values such as the terminal agent. Because `Session` flows through `ora-domain`, `ora-db`, application handlers, and tests, the change is small in data shape but cross-cutting in Rust APIs.

The database schema already stores `agent_id` as `TEXT`, and the public contract layer also models agent IDs as strings. That means the main design constraint is to strengthen the internal Rust model without forcing a persistence or transport migration.

## Goals / Non-Goals

**Goals:**
- Introduce a domain-owned `AgentId` newtype for `Session.agent_id`.
- Keep serde and SQLite representations string-compatible so existing persisted rows remain valid.
- Centralize known built-in agent identifiers behind the new domain type.
- Keep boundary conversions explicit so repository and application code remain easy to follow and test.

**Non-Goals:**
- Adding validation rules for arbitrary agent ID strings in this change.
- Changing the SQLite schema or stored session row format.
- Reworking public request and response DTOs beyond the minimum conversion updates needed to compile.
- Generalizing `AgentId` into a broader agent registry or lifecycle model.

## Decisions

### Introduce `AgentId` as a transparent domain newtype around `String`

`ora-domain` will define `AgentId` as a `#[serde(transparent)]` newtype stored alongside the session model, and `Session.agent_id` plus `Session::new` will adopt that type. The newtype will carry derives needed by the current domain usage and will expose the known terminal identifier as a domain-owned constant.

Why this approach:
- It makes the domain meaning explicit at every `Session` callsite without changing the persisted or serialized wire shape.
- It gives the codebase one stable place to add future agent-specific helpers or invariants without continuing to spread raw strings.

Alternatives considered:
- Keeping `String` and adding free constants only. Rejected because constants alone do not prevent accidental mix-ups with unrelated text and do not improve API clarity at the domain boundary.
- Replacing the field with an enum of known agents. Rejected because sessions can already refer to non-terminal agent identities, so an enum would either be incomplete or would need a catch-all variant that still reintroduces string handling complexity.

### Keep persistence and transport boundaries string-based, with conversion at the domain edges

SQLite session rows will continue to store `agent_id` as `TEXT`, and transport-facing DTOs can remain string-shaped. Repository mapping and any handler or mapper code that constructs domain sessions will convert between strings and `AgentId` explicitly.

Why this approach:
- It avoids a schema migration and keeps serialized payload compatibility stable.
- It localizes the typing change to the internal Rust layers that benefit most from the stronger model.

Alternatives considered:
- Migrating the database or contracts to a structured agent representation in the same change. Rejected because the requested improvement is about internal type safety, not external payload redesign.
- Hiding conversions behind trait objects or adapter-local wrapper types. Rejected because straightforward constructor and accessor usage is simpler and aligns with the repository's preference for static, explicit Rust APIs.

### Limit the first slice to session ownership and tests

The change will update `ora-domain`, `ora-db`, and directly affected application or contract callsites, but it will not broaden into unrelated model cleanup. Tests that build whole `Session` values will be updated to use `AgentId` so the stronger type is exercised end to end.

Why this approach:
- It keeps the change small enough to review while still covering every layer that currently constructs session snapshots.
- It lets follow-up changes expand agent identity semantics from a typed foundation instead of mixing design cleanup with feature work.

Alternatives considered:
- Refactoring all agent-related string fields across the repository at once. Rejected because only `Session.agent_id` is currently in scope and broadening the surface would add noise without clear product value.

## Risks / Trade-offs

- [Typed `Session` APIs may ripple through many tests and constructors] → Mitigation: update helpers and fixtures near the callsites so the conversion stays obvious and repeated boilerplate stays low.
- [Leaving contracts as strings can create temporary asymmetry between domain and transport models] → Mitigation: keep mapper boundaries explicit and document that the string wire shape is intentional compatibility, not an oversight.
- [No validation means invalid custom IDs can still be constructed] → Mitigation: keep the newtype lightweight now and reserve validation rules for a later change once the accepted identifier space is better defined.

## Migration Plan

1. Add `AgentId` to `ora-domain` and switch `Session` plus session-focused tests to the typed field.
2. Update SQLite repository reads and writes to encode and decode `AgentId` without changing the `sessions.agent_id` column shape.
3. Update any application or contract mappers that build or expose `Session` values so the domain change compiles cleanly.
4. Refresh relevant docs if API-facing or architecture docs reference session agent identity as a raw string.
5. Verify with formatting and the existing test suite; no data migration or rollback step is required because persistence remains string-compatible.

## Open Questions

None.
