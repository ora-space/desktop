### Requirement: Domain entities mirror the schema tables
The Rust domain layer SHALL define first-class models for every table declared in `docs/schema.sql`, including projects, tasks, worktrees, virtual folders, virtual entries, sessions, and artifacts.

#### Scenario: Domain crate exposes schema-backed entities
- **WHEN** a developer depends on the domain crate
- **THEN** they can construct and use typed models for each schema table without importing transport-specific or persistence-specific modules

### Requirement: Numeric category fields use enums in the domain layer
The domain layer SHALL represent numeric category columns with Rust enums instead of raw numeric fields. This includes task status, worktree active state, virtual entry kind, and session status.

#### Scenario: Callers inspect categorical state
- **WHEN** application or transport code reads a domain model with a categorical field
- **THEN** it receives an enum value that can be exhaustively matched instead of an integer code

### Requirement: Invalid categorical values are rejected at the boundary
The system SHALL keep numeric encoding and decoding logic outside the core domain models and MUST reject unknown persisted numeric values during conversion into domain enums.

#### Scenario: Persistence layer decodes unknown status code
- **WHEN** adapter code attempts to convert an unsupported numeric category value into a domain enum
- **THEN** the conversion fails explicitly instead of constructing a domain model with an invalid state

### Requirement: Optionality matches schema nullability
The domain models SHALL use optional fields only for columns that are nullable in `docs/schema.sql`, and SHALL keep non-nullable columns required in constructors and struct fields.

#### Scenario: Caller constructs a required entity
- **WHEN** a caller creates a model for a schema row with required columns
- **THEN** the type requires all non-nullable fields to be present and only allows absence for nullable columns

### Requirement: Session agent identifiers SHALL use a dedicated domain type
The domain layer SHALL represent `Session.agent_id` with a dedicated `ora_domain::AgentId` value instead of a raw string. `AgentId` SHALL remain string-compatible for serde boundaries, and the domain layer SHALL expose the known terminal session agent identifier through that type so callers can depend on a canonical built-in value.

#### Scenario: Caller constructs a session with a known built-in agent
- **WHEN** application or adapter code creates an `ora_domain::Session` for the terminal-backed flow
- **THEN** the session stores its agent identity as `ora_domain::AgentId` and can use the canonical terminal identifier exported by the domain layer instead of repeating a raw `"terminal"` literal

#### Scenario: Session is serialized through an existing string-based boundary
- **WHEN** a `Session` containing an `AgentId` is serialized or deserialized with serde
- **THEN** the `agent_id` field remains encoded as the same string value expected by existing boundaries rather than introducing a nested object shape
