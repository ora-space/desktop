## Why

Ora already has task-scoped sessions, backend-owned task worktrees, and a web server that fronts those application flows, but it does not yet provide an interactive terminal that can attach to a task workspace. The discussion in `docs/BRAINSTORM.md` converged on a task terminal backed by server-owned PTY runtime state, and now is the right time to formalize that direction so the terminal transport, session contracts, and backend boundaries evolve together instead of as adapter-specific shortcuts.

## What Changes

- Add a new Rust crate, `ora-pty`, that owns PTY process lifecycle, input/output streaming, resize handling, single-client attachment rules, and bounded in-memory output replay for terminal sessions.
- Add a task-terminal runtime flow that creates a persisted `Session` first, resolves the task-owned worktree on the server, starts the PTY process, and keys the live runtime by `session_id`.
- Extend session-facing contracts so terminal session creation can carry startup dimensions without exposing filesystem paths or transport-specific process details to callers.
- Add a dedicated terminal attach surface in the web runtime for streaming PTY output and terminal control messages over WebSocket while keeping create/get/list/update/delete session routes intact.
- Keep the first slice intentionally task-scoped: project-level terminals, persisted terminal scrollback, and server-side cell-diff rendering are out of scope.

## Capabilities

### New Capabilities
- `task-terminal-runtime`: Task-scoped terminal lifecycle, PTY runtime ownership, buffered reconnect replay, and terminal attach protocol behavior.

### Modified Capabilities
- `web-server-runtime`: Add terminal session startup wiring and a terminal WebSocket attach route alongside the existing HTTP runtime.
- `app-contracts`: Extend session contracts with terminal creation inputs and terminal transport message types while keeping worktree ownership internal to the backend.
- `application-handlers`: Add terminal-oriented application/service ports so the web runtime can create and manage task-backed terminal sessions without pushing PTY orchestration into adapters.

## Impact

- Affected code will span `crates/contracts`, `crates/application`, the new `crates/pty`, and `apps/web/server`.
- The web server will gain a WebSocket terminal attach endpoint and runtime-managed PTY state keyed by persisted session identifiers.
- New dependencies will likely include `portable_pty`, with terminal rendering intentionally left to the frontend `xterm.js` client instead of adding a server-side terminal emulator.
