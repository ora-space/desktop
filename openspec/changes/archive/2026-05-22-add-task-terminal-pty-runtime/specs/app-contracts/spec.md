## ADDED Requirements

### Requirement: Session contracts SHALL support terminal session startup inputs without exposing worktree paths
The system SHALL allow `CreateSessionRequest` to carry optional terminal startup parameters for terminal-backed sessions while keeping worktree resolution internal to the backend. The terminal startup payload SHALL include `cols` and `rows` in the first version, and non-terminal session creation SHALL omit terminal startup parameters entirely.

#### Scenario: Adapter submits a terminal session creation request
- **WHEN** an HTTP or Tauri adapter constructs a terminal-backed `CreateSessionRequest`
- **THEN** the request can include terminal startup dimensions and still omits any caller-supplied filesystem path or worktree identifier override

#### Scenario: Adapter submits a non-terminal session creation request
- **WHEN** an adapter constructs a non-terminal `CreateSessionRequest`
- **THEN** the request omits terminal startup parameters and continues to use the shared session contract shape

### Requirement: App contracts SHALL define shared terminal stream message types
The system SHALL expose serialization-friendly terminal message DTOs in `ora-contracts` for the task-terminal attach flow. The client-to-server message set SHALL include terminal input, resize, and explicit kill requests. The server-to-client message set SHALL include terminal ready, buffered history replay, live output, process exit, and terminal-scoped error events.

#### Scenario: Frontend sends terminal control messages
- **WHEN** a frontend client needs to write keystrokes, resize the viewport, or request terminal shutdown
- **THEN** it uses shared terminal message DTOs from `ora-contracts` instead of adapter-local ad hoc payloads

#### Scenario: Frontend consumes terminal stream events
- **WHEN** a frontend client receives terminal readiness, history replay, output, exit, or error data
- **THEN** the payload shape matches the shared terminal message DTOs exported from `ora-contracts`

### Requirement: Session update contracts SHALL keep terminal runtime-only state out of CRUD replacement payloads
The system SHALL keep runtime-only terminal control fields such as resize operations out of `UpdateSessionRequest`. Terminal viewport changes SHALL flow through terminal stream messages instead of session CRUD replacement semantics.

#### Scenario: Adapter needs to resize a running terminal
- **WHEN** an adapter or frontend client changes the terminal viewport size
- **THEN** it sends a terminal resize message rather than updating the persisted session through `UpdateSessionRequest`

### Requirement: Terminal creation dimensions SHALL define initial PTY size only
The system SHALL treat terminal `cols` and `rows` on `CreateSessionRequest` as the initial PTY dimensions used at startup time only. Reattaching to an existing running terminal session SHALL NOT create a new PTY from those initial values. Any later viewport-size changes SHALL flow through terminal resize messages after attach.

#### Scenario: Client reconnects with a different viewport size
- **WHEN** a client reattaches to an existing running terminal session from a viewport whose size differs from the dimensions used at session creation
- **THEN** the session is reattached to the existing PTY and the client updates the runtime terminal size by sending a resize message instead of recreating the session
