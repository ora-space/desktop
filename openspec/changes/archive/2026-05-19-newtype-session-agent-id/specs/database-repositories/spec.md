## ADDED Requirements

### Requirement: Session repository mappings SHALL preserve typed agent identifiers
The SQLite session repository SHALL map the persisted `sessions.agent_id` text column to and from `ora_domain::AgentId` when creating, updating, finding, and listing `Session` values. Repository callers SHALL receive typed session agent identities without any database schema change.

#### Scenario: Session row is loaded from SQLite with an agent identifier
- **WHEN** `ora-db` reads a visible `sessions` row
- **THEN** it converts the persisted `agent_id` text into `ora_domain::AgentId`, preserves the existing `agent_session_id` and status semantics, and returns a full `ora_domain::Session`

#### Scenario: Typed session is written back to SQLite
- **WHEN** `ora-db` creates or updates a `Session` whose `agent_id` is an `ora_domain::AgentId`
- **THEN** the repository persists the same underlying string value into the `sessions.agent_id` column without requiring a schema migration or adapter-local shadow field
