## Purpose

Define the PTY-backed runtime behavior for persisted task terminal sessions, including startup, attach/replay semantics, and shutdown coordination.

## Requirements

### Requirement: Task terminal runtime SHALL own one PTY-backed runtime per running terminal session
The system SHALL define a task-terminal runtime capability that starts a PTY process for a persisted terminal `Session`, resolves the terminal working directory from the task-owned backend worktree, and keys the live runtime state by `session_id`. The runtime SHALL treat the persisted `Session` as the durable identity while keeping PTY handles, process IO, and attachment state in memory only. If PTY startup fails after the session is persisted, the system SHALL retain that session and mark it `Stopped`.

#### Scenario: Terminal session starts successfully for a task
- **WHEN** the application starts a terminal session for a valid task-backed worktree
- **THEN** it persists or reuses the terminal `Session`, spawns one PTY process with that task worktree as the server-resolved working directory, and stores the live runtime under the returned `session_id`

#### Scenario: Terminal session startup cannot resolve a task worktree
- **WHEN** terminal startup is requested for a task whose backend-owned worktree cannot be resolved
- **THEN** the startup flow fails with a stable terminal-startup error and does not expose filesystem lookup details through the public contract

#### Scenario: PTY startup fails after the session is created
- **WHEN** the system has already persisted a terminal session and PTY startup fails before the runtime becomes healthy
- **THEN** the persisted session remains available in `Stopped` status and the startup flow returns a stable terminal-startup failure

### Requirement: Task terminal runtime SHALL preserve bounded replay history only while the PTY is running
The system SHALL buffer terminal output in append order for each running terminal session, SHALL bound that buffer by a runtime-owned trimming policy, and SHALL replay buffered output to a reconnecting client before forwarding new live output. The system SHALL allow reattachment to a still-running terminal for the lifetime of the server process without an idle timeout. WebSocket attachment and detachment SHALL NOT control PTY lifetime. The system SHALL clear the in-memory buffer when the PTY exits and the persisted session becomes stopped.

#### Scenario: Client reconnects to a running terminal session
- **WHEN** a client attaches to an already running terminal session after a previous socket disconnect
- **THEN** the runtime replays buffered output in append order before sending new live output from the PTY

#### Scenario: Client reconnects with a new viewport size
- **WHEN** a client reattaches to a running terminal session after the terminal was originally created with different `cols` and `rows`
- **THEN** the runtime keeps the existing PTY alive across reattachment and applies the new viewport dimensions only after receiving a resize message

#### Scenario: Client disconnects from a running terminal session
- **WHEN** the attached WebSocket client disconnects while the PTY is still running
- **THEN** the PTY runtime remains alive, preserves replayable output according to its buffer policy, and remains available for a later reattachment

#### Scenario: Terminal process exits
- **WHEN** the PTY process for a running terminal session exits
- **THEN** the runtime marks the session as stopped, emits the terminal exit event, and clears the in-memory replay buffer for that session

### Requirement: Task terminal runtime SHALL allow at most one attached live client per session
The system SHALL treat the first implementation of a task terminal as a single-attached-client runtime. It SHALL reject a second concurrent attach attempt for the same running terminal session instead of multiplexing live output to multiple clients.

#### Scenario: Second client attempts to attach to the same running terminal
- **WHEN** one live client is already attached to a running terminal session
- **THEN** the runtime rejects a second attach attempt with a stable terminal-attachment failure

### Requirement: Task terminal runtime SHALL terminate PTYs only from terminal or server lifecycle events
The system SHALL terminate a PTY only when the terminal process exits normally, when a terminal kill operation is explicitly requested, or when server shutdown tears down the runtime. Client transport disconnects SHALL NOT be treated as PTY termination events.

#### Scenario: User exits the shell inside the PTY
- **WHEN** the shell or foreground process inside the PTY exits normally
- **THEN** the runtime treats the terminal session as exited, updates persisted state, and releases runtime resources

#### Scenario: Server shuts down with running terminal sessions
- **WHEN** the server begins shutdown while PTY runtimes are still active
- **THEN** the runtime tears down the PTYs as part of server-owned shutdown instead of waiting for WebSocket disconnect behavior to drive cleanup

### Requirement: Task terminal runtime SHALL coordinate cancellation with server-owned and session-owned tokens
The system SHALL root terminal runtime shutdown in one server-owned cancellation signal and SHALL derive one child cancellation signal per running terminal session. PTY reader tasks and WebSocket attachment loops for a session SHALL observe that session-owned signal. WebSocket disconnects SHALL detach the client without canceling the session-owned signal directly. PTY exit, explicit terminal kill, or server shutdown SHALL cancel the session-owned signal and drive runtime teardown.

#### Scenario: PTY exit cancels the session runtime
- **WHEN** a running PTY exits for a session with active runtime tasks
- **THEN** the session-owned cancellation signal is triggered so PTY reader and attachment loops stop and the session runtime can be torn down deterministically

#### Scenario: Server shutdown cancels all active terminal sessions
- **WHEN** the server-owned cancellation signal is triggered while multiple PTY sessions are running
- **THEN** each session-owned cancellation signal is canceled through that parent relationship and all active runtimes begin shutdown
