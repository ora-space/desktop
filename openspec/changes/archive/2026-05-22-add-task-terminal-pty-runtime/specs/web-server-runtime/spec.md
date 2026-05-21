## ADDED Requirements

### Requirement: Web server runtime SHALL expose terminal attach and control routes for session-backed task terminals
The system SHALL expose a dedicated terminal attach route at `/api/sessions/{sessionId}/terminal` for session-backed task terminals in addition to the existing session CRUD routes. The route SHALL upgrade exactly one client connection to the live terminal stream for the addressed session, and it SHALL reject unsupported or invalid attach attempts during route handling instead of accepting the socket and failing later with undefined behavior. The web runtime SHALL introduce this terminal WebSocket transport in a way that future Tauri clients can reuse the same session-scoped protocol. The WebSocket route SHALL attach to an already running PTY runtime rather than owning the PTY lifecycle directly.

#### Scenario: Client attaches to a running terminal session
- **WHEN** a caller opens `/api/sessions/{sessionId}/terminal` for a running terminal session with no currently attached client
- **THEN** the web runtime upgrades the connection and begins the ordered terminal attach flow for that session

#### Scenario: Client disconnects from an attached terminal session
- **WHEN** an attached WebSocket connection closes while the addressed PTY runtime is still running
- **THEN** the web runtime detaches that client session without terminating the PTY and allows a later reattachment by the same persisted `session_id`

#### Scenario: Client attaches to a non-terminal or stopped session
- **WHEN** a caller opens `/api/sessions/{sessionId}/terminal` for a session that is not a terminal session or is already stopped
- **THEN** the web runtime rejects the attach attempt with a stable terminal-specific failure response

#### Scenario: Client attempts a duplicate attach
- **WHEN** a caller opens `/api/sessions/{sessionId}/terminal` while another client is already attached to the same running terminal session
- **THEN** the web runtime rejects the second attach attempt instead of sharing the PTY stream

### Requirement: Web server runtime SHALL shut terminal WebSocket loops down through session cancellation rather than client disconnect semantics
The system SHALL wire terminal WebSocket loops to observe the owning session cancellation signal derived from the server shutdown signal. When session cancellation occurs because of PTY exit, explicit kill, or server shutdown, the WebSocket loop SHALL end safely after terminal-final handling. Any terminal-final or exit message delivery during shutdown SHALL be best-effort and SHALL NOT block runtime teardown waiting for WebSocket flush completion. Client disconnect alone SHALL detach the client without being treated as runtime shutdown.

#### Scenario: PTY exits while a client is attached
- **WHEN** the PTY for an attached terminal session exits while the WebSocket loop is active
- **THEN** the web runtime sends the terminal exit event, observes session cancellation, closes the attachment loop, and releases the client attachment state

#### Scenario: Server shutdown begins while a client is attached
- **WHEN** the server shutdown signal is triggered while a terminal WebSocket loop is active
- **THEN** the loop observes the derived session cancellation and exits safely as part of terminal runtime teardown

### Requirement: Web server runtime SHALL start task terminals through session creation without exposing backend worktree paths
The system SHALL keep terminal startup in the server-owned create-session flow. When the caller creates a terminal session, the runtime SHALL accept terminal startup dimensions, resolve the terminal working directory from the task-owned backend worktree, start the PTY runtime through application-owned services, and return the persisted session payload without exposing filesystem paths in the public API. If startup fails after the session is persisted, the runtime SHALL retain the session in `Stopped` status rather than deleting it.

#### Scenario: Client creates a terminal session successfully
- **WHEN** a caller submits a create-session request that identifies a terminal session and includes valid startup dimensions
- **THEN** the web runtime creates the persisted session, starts the PTY runtime for the task-owned worktree, and returns the shared session response payload

#### Scenario: Terminal startup fails during session creation
- **WHEN** the runtime cannot start the PTY after receiving a terminal session creation request
- **THEN** the web runtime returns a stable structured failure response, retains the created session in `Stopped` status, and does not leak PTY implementation details or raw filesystem errors
