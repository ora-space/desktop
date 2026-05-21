## 1. Add the `ora-pty` runtime crate

- [x] 1.1 Create the new `crates/pty` crate as `ora-pty`, add it to the workspace, and introduce the PTY/runtime dependencies needed for process spawning, IO streaming, and bounded output buffering.
- [x] 1.2 Define the `ora-pty` runtime API for terminal session startup, attach state, input, resize, kill, replay history, exit notifications, and the server-token/session-token cancellation model using testable Rust types and traits.
- [x] 1.3 Implement the first PTY runtime manager in `ora-pty`, including single-client attach enforcement, ordered history replay, no-idle-timeout reattachment for running sessions, PTY lifecycle independent from WebSocket disconnects, deterministic teardown ordering, runtime cleanup, and focused unit tests with fakes where possible.

## 2. Expand contracts and application services for task terminals

- [x] 2.1 Extend `crates/contracts/src/session.rs` with terminal startup request fields and shared terminal stream message DTOs, then re-export any new public contract types from `crates/contracts/src/lib.rs`.
- [x] 2.2 Add serialization-focused contract tests that cover terminal session creation payloads, terminal stream message JSON shapes, and the distinction between initial startup dimensions and later resize messages.
- [x] 2.3 Introduce terminal-oriented services or handlers plus ports in `crates/application` for task-terminal startup, attach, input, resize, kill, and exit-state synchronization without leaking adapter-specific types.
- [x] 2.4 Extend `ora-application` error handling and unit tests so terminal startup marks failed sessions as `Stopped`, duplicate attach, missing runtime, and PTY-exit persistence paths return stable outcomes and structured logs.

## 3. Wire the web server terminal runtime

- [x] 3.1 Construct and own the shared terminal runtime manager in `apps/web/server`, introducing the new terminal WebSocket wiring, the root server shutdown token, and the session child-token lifecycle needed to resolve task worktrees and persist session status changes.
- [x] 3.2 Extend the existing session creation flow so terminal-backed session requests start a PTY for the task-owned worktree without exposing filesystem paths in the public API.
- [x] 3.3 Add the `/api/sessions/{sessionId}/terminal` WebSocket attach route and implement the first-version terminal protocol for `ready`, `history`, `output`, `exit`, `error`, `input`, `resize`, and `kill`, keeping the protocol reusable for future Tauri clients and ensuring WebSocket disconnect only detaches the client instead of terminating the PTY.
- [x] 3.4 Add integration-style tests for terminal session creation, `Stopped` startup-failure compensation, attach rejection for invalid or duplicate sessions, reconnect history replay without idle timeout, reconnect with a different viewport followed by runtime resize, WebSocket disconnect without PTY termination, server-token-driven shutdown, and exit-driven session status updates.

## 4. Update docs and verify the slice

- [x] 4.1 Update the relevant `docs/` content to describe the new task-terminal session flow, `ora-pty` ownership boundary, the no-idle-timeout reconnect rule, the PTY/WS lifecycle decoupling, and the WebSocket terminal protocol shared by web and future Tauri clients.
- [x] 4.2 Run `cargo fmt --all` and `task test`, then fix any compile, test, or contract-generation regressions introduced by the terminal runtime change.
