# ACP Agent Runtime

This module owns the application-scoped runtime for supported agent CLIs and the serialized lifecycle actor for each persisted Ora session.

## Runtime model

- `AgentRuntimeManager` owns one independently supervised ACP child connection per supported CLI and routes sessions to the supervisor selected by their immutable `agent_cli`.
- Each session has one actor that serializes load, prompt, permission, cancellation, stop, and deletion commands.
- Sessions targeting the same CLI share its process and connection; sessions targeting different CLIs or different actors can progress concurrently.
- Model discovery runs each CLI's bounded command independently and returns only successful groups.

## Flow control and failure isolation

- The central connection router receives unbounded connection-wide updates, then forwards them into bounded per-session queues of 256 items.
- Session overflow, prompt timeout, or cancellation stops only the affected session. Connection framing, correlation, or stdio failure invalidates the connection generation and stops only sessions registered on that CLI.
- Control messages such as permission requests use a separate path so update backpressure cannot block required protocol responses.
- Routes are generation-bound. Updates from old connections or unloaded sessions are discarded as stale.

## Lifecycle boundaries

Startup reconciles stale persisted Running sessions to Stopped. Create persists only after `session/new` succeeds; load restores Stopped on setup failure. A session accepts only one load or prompt operation at a time.

Cancellation sends `session/cancel` and waits for bounded settlement. Explicit stop may call `session/close` when supported, unloads routing, and retains provider history. Deletion removes only Ora's stopped record after serialized unload; it does not delete provider history.

Supervisors retry failed providers independently with capped backoff and reap the old process tree before replacement. Ora remains available when one or all providers are unavailable.

See the [ora-backend overview](../../README.md) and [ACP Agent Runtime design](../../../../docs/agent-runtime.md).
