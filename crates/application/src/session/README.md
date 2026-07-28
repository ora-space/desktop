# Session Application Module

This module provides the persistence-facing portion of Ora session lifecycle.

## Responsibilities

- `GetSessionHandler` and `ListSessionsHandler` map visible domain sessions to shared contract responses.
- `DeleteSessionHandler` soft-deletes a persisted session record using the injected clock and returns a stable not-found error when necessary.
- `SessionRepository` defines create, read, list, update, and soft-delete operations used by application and backend composition.
- `SessionIdGenerator` provides injectable session identity generation for runtime-owned creation flows.

Session creation, provider handshake, load, prompt, permission response, cancellation, stop, and lifecycle serialization do not belong here. The backend agent runtime performs those operations and uses the repository port to persist state transitions.

Callers must stop and unload a running session before using the deletion handler. Provider-owned history is outside Ora's persistence boundary and is never deleted by this module.

See the [ora-application overview](../../README.md) and [ACP Agent Runtime](../../../../docs/agent-runtime.md).
