# Session Application Module

This module provides the persistence-facing portion of Ora session lifecycle.

## Responsibilities

- `GetSessionHandler` maps any visible domain session, including a workflow-owned node session, so Theater can load its transcript directly. `ListSessionsHandler` projects only standalone sessions; sessions bound to visible workflow node runs are intentionally absent from ordinary chat lists.
- `RenameSessionHandler` validates a user-supplied title, updates only the title column, and returns the latest session snapshot. A missing or soft-deleted row maps to `SessionNotFound` rather than a generic repository failure.
- `DeleteSessionHandler` soft-deletes a persisted session record using the injected clock and returns a stable not-found error when necessary.
- `SessionRepository` defines create, read, complete-list, standalone-list, soft-delete, and single-intent session mutations used by application and backend composition. The complete list remains available for lifecycle and history cleanup, while the standalone list owns the ordinary-chat projection boundary. Title, status, binding, and history-state updates each own only their columns and return the latest complete session snapshot.
- `SessionIdGenerator` provides injectable session identity generation for runtime-owned creation flows.

Session creation, provider handshake, load, prompt, permission response, cancellation, stop, agent switching, and lifecycle serialization do not belong here. The backend agent runtime performs those operations and uses the repository port to persist state transitions.

Callers must stop and unload a running session before using the deletion handler. Neither Ora's recorded session history nor the provider's own history is touched by this module: the history file belongs to the backend agent runtime, which removes it alongside the row, and provider-owned history is outside Ora's persistence boundary entirely.

Session mutations are intentionally split into `update_session_title`, `update_session_status`, `update_session_binding`, and `update_session_history_state`. This prevents an actor or connection supervisor holding an older snapshot from overwriting a newer title or lifecycle state. None of these operations moves a session to another task.

See the [ora-application overview](../../README.md) and [ACP Agent Runtime](../../../../docs/agent-runtime.md).
