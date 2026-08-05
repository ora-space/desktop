# ACP Agent Runtime

This module owns the application-scoped runtime for supported agent CLIs and the serialized lifecycle actor for each persisted Ora session.

## Runtime model

- `AgentRuntimeManager` owns one independently supervised ACP child connection per supported CLI and routes sessions to the supervisor selected by their current `agent_cli` binding. Switching replaces that binding while preserving the Ora session and its recorded history.
- Each session has one actor that serializes load, prompt, permission, cancellation, stop, and deletion commands.
- Sessions targeting the same CLI share its process and connection; sessions targeting different CLIs or different actors can progress concurrently.
- Prompts preserve the public ACP `ContentBlock` sequence, so one turn can contain text, images, audio, and linked or embedded resources instead of being reduced to plain text.
- Model discovery runs each CLI's bounded command independently and returns only successful groups.

## Flow control and failure isolation

- The central connection router receives unbounded connection-wide updates, then forwards them into bounded per-session queues of 256 items.
- While `session/new` is waiting to reveal its provider session id, the router temporarily buffers otherwise-unrouted setup updates. Registration drains matching updates into the new session route; unmatched setup updates are discarded when the last concurrent setup finishes.
- Session overflow, prompt timeout, or cancellation stops only the affected session. Connection framing, correlation, or stdio failure invalidates the connection generation and stops only sessions registered on that CLI.
- Control messages such as permission requests use a separate path so update backpressure cannot block required protocol responses.
- Routes are generation-bound. Updates from old connections or unloaded sessions are discarded as stale.

## Lifecycle boundaries

Startup reconciles stale persisted Running sessions to Stopped. Create persists only after `session/new` succeeds, opens Ora's history record before the first prompt, returns the latest setup-time available-command catalog, and retains other setup updates for the first prompt. Load restores Stopped on setup failure and streams Ora's recorded history rather than the provider's replay. A session accepts only one load or prompt operation at a time.

Prompt validation rejects an empty content-block sequence or one containing only blank text blocks. The serialized ACP prompt payload is limited to 16 MiB before it reaches the provider.

Cancellation sends `session/cancel` and waits for bounded settlement. Explicit stop may call `session/close` when supported, unloads routing, and retains provider history. A failed history write moves the session into a degraded state and refuses later prompts until history is resumed. Switching creates the new provider session before releasing the old binding, then injects the recorded transcript into the next prompt. Deletion removes Ora's stopped record and its Ora-owned history after serialized unload; it does not delete provider history.

Supervisors retry failed providers independently with capped backoff and reap the old process tree before replacement. Ora remains available when one or all providers are unavailable.

See the [ora-backend overview](../../README.md) and [ACP Agent Runtime design](../../../../docs/agent-runtime.md).
