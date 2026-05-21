## ADDED Requirements

### Requirement: Application terminal startup SHALL be orchestrated through transport-agnostic services and ports
The system SHALL define terminal-oriented application services or handlers inside `ora-application` that accept transport-neutral terminal startup requests, create or validate the persisted terminal session, resolve the task-owned worktree through application-owned dependencies, and start the PTY runtime through an application-owned port. This orchestration SHALL remain independent of HTTP, WebSocket, and concrete PTY implementation details.

#### Scenario: Web adapter starts a terminal session
- **WHEN** the web runtime receives a valid terminal session startup request
- **THEN** it can delegate the startup flow to one transport-agnostic application service and receive a stable session-backed result

#### Scenario: Unit test replaces terminal runtime dependencies
- **WHEN** a test constructs the terminal startup service with fake repositories, fake worktree resolution, and a fake PTY runtime dependency
- **THEN** the full startup flow can be exercised without a real PTY process or WebSocket server

### Requirement: Application terminal lifecycle SHALL keep session state and PTY runtime state consistent
The system SHALL require terminal lifecycle services to synchronize persisted session state with runtime events. Terminal startup failures SHALL return stable application errors without leaving undefined runtime state, and PTY exit handling SHALL transition the persisted session status to stopped through application-owned dependencies. When startup fails after session persistence, the compensation behavior SHALL retain the session and mark it stopped. Lifecycle orchestration SHALL preserve deterministic teardown order: terminal-final signaling on a best-effort basis, client detachment, runtime teardown, and resource release.

#### Scenario: PTY startup fails after session persistence begins
- **WHEN** the terminal startup flow cannot finish creating the PTY runtime
- **THEN** the application service returns a stable startup failure and marks the persisted session as stopped so it does not remain indistinguishable from a healthy running terminal

#### Scenario: PTY exits after a running terminal session starts
- **WHEN** the PTY runtime reports process exit for a running terminal session
- **THEN** the application-owned terminal lifecycle flow updates the persisted session status to stopped and releases the associated runtime state

#### Scenario: Server shutdown tears down a running terminal session
- **WHEN** the terminal lifecycle flow receives a server-driven shutdown for a running terminal session
- **THEN** it drives the same deterministic teardown path used for PTY-final lifecycle completion and leaves the persisted session in a non-running state

### Requirement: Application terminal attach and control SHALL stay behind application-owned abstractions
The system SHALL define application-owned abstractions for terminal attach, terminal input, terminal resize, and terminal kill operations so adapters do not manipulate PTY runtime internals directly. These abstractions SHALL return stable outcomes for missing sessions, stopped sessions, and duplicate attachments, and they SHALL remain transport-neutral so both web and future Tauri adapters can drive the same terminal lifecycle over a shared WebSocket protocol.

#### Scenario: Adapter attaches to a running terminal
- **WHEN** an adapter requests attachment for a running terminal session
- **THEN** it receives an application-level terminal attachment result rather than direct access to PTY internals

#### Scenario: Adapter sends terminal control to a missing session
- **WHEN** an adapter sends input, resize, or kill for a session whose terminal runtime does not exist
- **THEN** the application abstraction returns a stable terminal-specific failure outcome
