## ADDED Requirements

### Requirement: Session agent identifiers SHALL use a dedicated domain type
The domain layer SHALL represent `Session.agent_id` with a dedicated `ora_domain::AgentId` value instead of a raw string. `AgentId` SHALL remain string-compatible for serde boundaries, and the domain layer SHALL expose the known terminal session agent identifier through that type so callers can depend on a canonical built-in value.

#### Scenario: Caller constructs a session with a known built-in agent
- **WHEN** application or adapter code creates an `ora_domain::Session` for the terminal-backed flow
- **THEN** the session stores its agent identity as `ora_domain::AgentId` and can use the canonical terminal identifier exported by the domain layer instead of repeating a raw `"terminal"` literal

#### Scenario: Session is serialized through an existing string-based boundary
- **WHEN** a `Session` containing an `AgentId` is serialized or deserialized with serde
- **THEN** the `agent_id` field remains encoded as the same string value expected by existing boundaries rather than introducing a nested object shape
