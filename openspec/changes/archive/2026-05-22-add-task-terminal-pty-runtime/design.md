## Context

`docs/BRAINSTORM.md` already narrowed the desired terminal architecture for the web frontend: the browser should render with `xterm.js`, the server should keep task worktree ownership private, and the persisted `Session` model should become the durable identity for task terminals. The repository also already has the pieces that make this practical: task-scoped sessions in `ora-domain`, backend-owned task worktree provisioning, transport-neutral application handlers, and a web runtime that wires HTTP routes on top of shared services.

What is missing is the runtime layer that owns a PTY process after session creation. That runtime does not fit existing CRUD modules cleanly because it combines persisted state, long-lived in-memory process handles, bidirectional streaming, disconnect/reconnect behavior, and terminal-specific lifecycle rules. The user also explicitly wants this work anchored by a new `crates/pty` crate, which makes the change cross-cutting rather than a small route addition.

## Goals / Non-Goals

**Goals:**

- Add a dedicated `ora-pty` crate that encapsulates PTY spawning, IO streaming, resize requests, termination, and bounded replay history behind Rust APIs that other crates can test against.
- Introduce a task-terminal application flow that creates or validates a persisted terminal `Session`, resolves the task-owned worktree on the server, starts the PTY, and tracks the live runtime by `session_id`.
- Extend `ora-contracts` so terminal session creation can describe startup dimensions and so terminal transport messages have shared DTOs for adapters and frontend generation.
- Add a web runtime terminal attach route that upgrades one session-scoped connection to a WebSocket and streams ordered terminal events with stable error handling.
- Keep the design testable with application-owned ports and in-memory fakes, following the repo's existing DfT conventions.

**Non-Goals:**

- Project-scoped terminals or terminals that are not bound to a task-owned worktree.
- Persisting terminal scrollback or PTY state across process restarts.
- Server-side terminal cell rendering, ANSI parsing, or `wezterm_term` integration.
- Replacing existing generic session CRUD routes with a terminal-only resource model.
- Designing the full frontend terminal UI beyond the transport and contract needs required for implementation.

## Decisions

### Add `ora-pty` as a runtime-focused crate instead of embedding PTY logic in the web server

`ora-pty` will own PTY lifecycle concerns: process spawn configuration, reader/writer task orchestration, resize and kill commands, runtime events, and bounded in-memory replay storage. The crate will expose testable traits and plain Rust types so `ora-application` or `ora-web-server` can compose it without importing terminal internals everywhere.

Why:
- It keeps PTY-specific dependencies and concurrency code out of the web adapter crate.
- It creates a reusable boundary for future Tauri or alternate runtime consumers without prematurely baking WebSocket details into the PTY layer.
- It aligns with the user's explicit requirement to add a new crate under `crates/pty`.

Alternative considered:
- Put PTY runtime code directly in `apps/web/server`.
  Rejected because it would blur adapter and runtime ownership, make testing harder, and make the future Tauri reuse path more expensive.

### Keep persisted `Session` as the durable terminal identity and key live runtime state by `session_id`

Task terminals will reuse the existing session model, with terminal sessions identified by convention such as `agent_id = "terminal"` and `agent_session_id = None` in the first slice. The session row remains the durable lookup key, while `ora-pty` keeps PTY handles, attachment state, and replay history in memory under the same `session_id`.

Why:
- The existing model already captures the task-scoped identity the terminal needs.
- Reusing `Session` avoids inventing a second durable terminal table before the first workflow exists.
- It gives reconnect and exit-status updates a stable identifier that already fits current HTTP/session routes.

Alternative considered:
- Add a separate terminal-specific persisted entity now.
  Rejected because it would duplicate existing session semantics and expand the migration surface before real product pressure exists.

### Create terminal sessions through HTTP first, then attach over WebSocket

Terminal startup arguments will be accepted as part of session creation, and the WebSocket route will only handle attach and runtime control. The create flow will validate task ownership and terminal startup data, create the persisted session, start the PTY, and only then return the created session. If PTY startup fails after session persistence, the compensating path will retain the session and mark it `Stopped` for diagnostics. The attach route will target `/api/sessions/{sessionId}/terminal` and represent exactly one live client in the first version.

Why:
- It fits the repository's existing request/response contract style and generated SDK model.
- It keeps launch failures in a normal HTTP response boundary rather than overloading the first WebSocket frame with validation semantics.
- It prevents the attach protocol from needing to create durable state implicitly.

Alternative considered:
- Create the terminal session lazily from the first WebSocket message.
  Rejected because it mixes validation, persistence, and streaming concerns into one transport path and makes startup errors less consistent.

### Let the browser render raw PTY output and keep the server protocol text-oriented

The server will stream text chunks from the PTY as terminal output and replay buffered text chunks in append order after attach. Terminal control messages will stay small and explicit: client `input`, `resize`, `kill`; server `ready`, `history`, `output`, `exit`, and terminal-scoped `error`.

Why:
- `xterm.js` already understands terminal escape sequences, so server-side cell diff rendering would duplicate work.
- Append-ordered text replay keeps reconnect logic deterministic and simpler to test.
- A narrow protocol reduces coupling between Rust internals and frontend renderer details.

Alternative considered:
- Reuse a screen-diff or cell-grid protocol inspired by `lunel/pty`.
  Rejected because that protocol fits a custom renderer better than a browser-side VT terminal and would add more moving parts than this repository needs initially.

### Treat terminal creation dimensions as initial runtime state and later viewport changes as resize messages

The `cols` and `rows` provided during terminal session creation define only the initial PTY size at spawn time. Reattaching to an existing running terminal session SHALL NOT recreate the PTY or reapply session creation semantics. If a reconnecting client has a different viewport size, it updates the running PTY by sending a terminal `resize` message after attach. Runtime terminal dimensions are therefore mutable session runtime state, not immutable session metadata.

Why:
- It preserves the identity of the running shell instead of replacing it when clients reconnect with different viewport sizes.
- It matches how browser and desktop clients naturally learn their actual viewport only at attach time.
- It keeps resize behavior in the live terminal protocol where it belongs instead of smuggling it into CRUD replacement flows.

Alternative considered:
- Treat a new viewport size on reconnect as a reason to recreate the PTY or a reason to mutate persisted session metadata.
  Rejected because it would break reconnect expectations and confuse durable session identity with runtime-only terminal state.

### Standardize terminal transport on WebSocket for both web and future Tauri clients

The first implementation will introduce a dedicated WebSocket transport in the runtime wiring by using `axum::extract::ws` in `apps/web/server`, and that same session-scoped terminal protocol will be the intended transport for future Tauri support instead of creating a second terminal-specific bridge.

Why:
- One terminal transport keeps the runtime, protocol tests, and client semantics aligned across surfaces.
- It avoids splitting PTY lifecycle behavior across separate web and Tauri integration layers.
- It preserves `ora-application` as transport-agnostic while still allowing adapters to converge on one live-stream transport.

Alternative considered:
- Use WebSocket for web now and introduce a Tauri-specific terminal bridge later.
  Rejected because it would duplicate protocol work and raise the risk that reconnect, control, and error semantics drift between clients.

### Decouple PTY lifecycle from WebSocket attachment lifecycle

The PTY runtime and the WebSocket attachment loop will be separate concerns. The PTY starts during terminal session creation and continues running independently of any individual WebSocket connection. A WebSocket connection only attaches to or detaches from an existing running PTY runtime. WebSocket disconnects, browser refreshes, or Tauri reconnects SHALL NOT terminate the PTY. The PTY exits only when the shell or user session exits normally, when an explicit terminal kill flow is invoked, or when the server shuts down and tears down runtime state.

Why:
- It matches the intended reconnect behavior and avoids losing terminal state on ordinary client disconnects.
- It keeps terminal ownership on the server instead of coupling process lifetime to a fragile client transport.
- It gives web and future Tauri clients the same attach/detach semantics over one shared protocol.

Alternative considered:
- Couple PTY lifetime to the currently attached WebSocket session.
  Rejected because transient client disconnects would destroy useful terminal state and make reconnect semantics impossible.

### Coordinate shutdown with a server token and per-session child tokens

The runtime will use one server-scoped `CancellationToken` as the root shutdown signal. Each running PTY session will own a child token derived from that server token. PTY reader tasks and WebSocket attachment loops will observe the session token, but WebSocket disconnects will only detach the client and SHALL NOT cancel the token directly. Session-token cancellation happens when the PTY exits, when an explicit kill flow terminates the terminal, or when server shutdown cancels the root token. Cleanup order will be deterministic: PTY exit or server shutdown triggers session-token cancellation, terminal exit is broadcast to attached clients on a best-effort basis, clients are detached, session runtime state is torn down, and resources are freed. Socket delivery during shutdown is opportunistic and SHALL NOT delay teardown completion.

Why:
- A token tree matches the real ownership model: the server owns all sessions, and each session owns its PTY and attachment tasks.
- It keeps PTY readers and WebSocket loops responsive to the same shutdown signal without coupling client disconnects to process termination.
- It gives server shutdown one clear control point for draining all active terminal runtimes.

Alternative considered:
- Manage PTY readers, WebSocket loops, and shutdown with unrelated ad hoc channels per task.
  Rejected because it would make ownership and teardown ordering harder to reason about and test consistently.

### Keep terminal orchestration transport-agnostic in `ora-application`

The application layer will gain terminal-oriented services or handlers plus ports for session persistence, task/worktree lookup, PTY runtime control, and exit-state synchronization. The web server will wire HTTP and WebSocket routes to those services, but it will not own the orchestration rules itself.

Why:
- This preserves the repo's pattern that business workflows live behind application-owned abstractions.
- Unit tests can exercise startup, attach, and exit transitions with in-memory fakes instead of a real PTY or socket server.
- It avoids baking WebSocket assumptions into all terminal lifecycle logic.

Alternative considered:
- Treat terminal startup as adapter glue around existing CRUD handlers only.
  Rejected because the create/attach lifecycle includes domain rules and compensating behavior that should not live in route code.

### Keep replay history bounded and runtime-only

`ora-pty` will store terminal output in a per-session in-memory ring buffer or chunk buffer with trimming based on total buffered bytes or chunk count. History exists only while the PTY is running. Running terminals remain reattachable for the full lifetime of the server process with no idle timeout in the first version. Once the PTY exits and the session status is moved to `Stopped`, runtime state and replay history are cleared.

Why:
- It supports reconnect without turning terminal output into persisted application data.
- It bounds memory usage for noisy sessions.
- It keeps the first implementation operationally simple and avoids migration work.

Alternative considered:
- Persist replay history in SQLite.
  Rejected because it would add storage design, retention policy, and migration complexity before reconnect behavior is validated.

## Risks / Trade-offs

- [Persisting the session before PTY startup can leave a just-created session without a live runtime if spawn fails mid-flow] -> Mitigation: keep the persisted session for diagnostics, mark it `Stopped`, and return a stable startup failure so callers can distinguish failed startup from missing state.
- [Single-client attach simplifies ownership but blocks concurrent viewers] -> Mitigation: document it as an explicit first-slice constraint and keep attachment state isolated so a later multi-viewer change can evolve the rule intentionally.
- [In-memory replay is lost on server restart] -> Mitigation: keep reconnect promises scoped to the current server process and expose stopped or missing-session failures clearly.
- [PTY runtime code adds concurrency and process-lifecycle complexity] -> Mitigation: isolate it inside `ora-pty`, design trait-based seams for tests, and keep the wire protocol minimal.
- [Using `agent_id = "terminal"` is a convention rather than a typed enum today] -> Mitigation: centralize the convention in contracts and startup services so future typing improvements only change one boundary.

## Migration Plan

1. Add the new `ora-pty` crate and define runtime-facing types, traits, and tests around PTY session lifecycle and buffered output.
2. Extend `ora-contracts` with terminal session create inputs and terminal message DTOs, then export the generated frontend types.
3. Add terminal-oriented services and ports in `ora-application`, including session startup, runtime attachment, control operations, and exit-state persistence.
4. Wire the web server runtime to instantiate the terminal manager, introduce the dedicated terminal WebSocket transport, expose the attach route, and translate terminal failures into stable HTTP/WebSocket responses in a way that future Tauri clients can reuse unchanged.
5. Validate with `cargo fmt --all` and `task test`, plus focused integration tests for session creation, attach rejection, reconnect replay, and PTY exit transitions.

Rollback strategy:
- Revert the terminal route wiring and stop constructing the PTY runtime manager.
- Because the first slice reuses existing `Session` persistence rather than adding schema changes, rollback does not require a database migration reversal.

## Open Questions

- None at the design level for the first implementation slice.
