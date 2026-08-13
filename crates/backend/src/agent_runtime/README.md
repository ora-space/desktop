# ACP Agent Runtime

This module owns the application-scoped runtime for supported agent CLIs and the serialized lifecycle actor for each persisted Ora session.

## Runtime model

- `AgentRuntimeManager` owns one independently supervised ACP child connection per supported CLI and routes sessions to the supervisor selected by their current `agent_cli` binding. Switching replaces that binding while preserving the Ora session and its recorded history.
- Each session has one actor that serializes load, prompt, permission, cancellation, stop, and deletion commands.
- Sessions targeting the same CLI share its process and connection; sessions targeting different CLIs or different actors can progress concurrently.
- Prompts preserve the public ACP `ContentBlock` sequence, so one turn can contain text, images, audio, and linked or embedded resources instead of being reduced to plain text.
- Model discovery runs each CLI's bounded command independently and returns only successful groups.
- CLI executables are located with the same semantics on every platform: each directory on the process `PATH` is searched first, then the CLI's fixed per-user install directory (`~/.{cli}/bin`) as a fallback. PATH wins so the resolved binary matches what `which` reports in the user's terminal; the fallback keeps official install-script setups working when a desktop-launched GUI process inherits a minimal PATH. Known limitation: PATH entries added only by shell rc files (nvm, bun) may be invisible to a GUI-launched process; the not-found error enumerates every searched location to keep that diagnosable.
- Session cwd for project-root tasks and warm project-root chats is resolved against the bootstrap path base, not live process cwd.
- A newly attached session has a non-persisted title-acquisition window. It accepts valid ACP `session_info_update` titles from attach onward and, after the first eligible prompt (`EndTurn`, `MaxTokens`, or `MaxTurnRequests`), schedules bounded `session/list` fallbacks at three and ten seconds when the CLI advertises that capability. Restored sessions start with acquisition disabled.

## Flow control and failure isolation

- The central connection router receives one unbounded, ordered connection-wide event stream, then forwards updates, permission requests, and terminating responses into a bounded per-session FIFO of 256 items.
- While `session/new` is waiting to reveal its provider session id, the router temporarily buffers otherwise-unrouted setup updates. Registration drains matching updates into the new session route; unmatched setup updates are discarded when the last concurrent setup finishes. Permission requests and responses are never treated as setup updates and are cancelled or discarded when no route exists.
- An active actor grants every permission request it receives immediately, preferring the offered option that also remembers the choice for later calls in the same turn. Ora runs with no human-in-the-loop review of tool calls; the `RespondToPermission` command remains for a request that arrives outside an active prompt, where it fails with "not pending" since nothing was left unanswered.
- Session overflow, prompt timeout, or cancellation stops only the affected session. Connection framing, correlation, or stdio failure invalidates the connection generation and stops only sessions registered on that CLI.
- Connection loss and queue overflow use an independent control channel, so a terminal failure cannot be hidden behind the bounded event FIFO. An active actor drains already accepted events before applying that control and polls its command channel after a bounded event burst.
- A load or prompt completes only after consuming its matching response fence. This keeps updates that precede the response in the same operation and prevents tail events from leaking into the next prompt.
- Routes are generation-bound. Updates from old connections or unloaded sessions are discarded as stale.
- Opening a session route also registers its provider-to-Ora identity mapping, so ACP frame traces correlate on the stable Ora session id.
- Title-list polling is actor-owned low-priority work: scheduler tasks only enqueue internal commands, the actor performs the ACP request when idle, and a new user operation can preempt it. Each list request has a five-second cap; the final attempt closes the window regardless of its result. `TitleUpdate` does not terminate an in-flight list request, while stale prompt-stream `Cancel` commands are ignored. Slow consumers cannot block the session actor or the application-event publisher.

## Lifecycle boundaries

Startup reconciles stale persisted Running sessions to Stopped. Create persists only after `session/new` succeeds, opens Ora's history record before the first prompt, returns the latest setup-time available-command catalog, and retains other setup updates for the first prompt. Load restores Stopped on setup failure and streams Ora's recorded history rather than the provider's replay. A session accepts only one load or prompt operation at a time.

Every provider session this module binds is obtained from the warm pool, which alone decides when one is created and how an unused one is released. Persisting a new session claims its warm entry by identifier; rebinding an existing session onto another CLI claims by key, because that identifier belongs to the pool and only the binding moves. Both reserve before mutating anything, so a failure returns the entry instead of stranding a provider session no bound would reclaim. Rebinding stops the outgoing binding's actor, which is why callers are expected to commit a move together with the next prompt rather than at the moment a CLI is chosen.

Prompt validation rejects an empty content-block sequence or one containing only blank text blocks. The serialized ACP prompt payload is limited to 16 MiB before it reaches the provider.

Cancellation sends `session/cancel` and waits for bounded settlement. Explicit stop may call `session/close` when supported, unloads routing, and retains provider history. A failed history write moves the session into a degraded state and refuses later prompts until history is resumed. Switching creates the new provider session before releasing the old binding, then injects the recorded transcript into the next prompt. Deletion removes Ora's stopped record and its Ora-owned history after serialized unload; it does not delete provider history.

Session titles are validated as the domain `SessionTitle` value object, persisted through a title-only repository operation taking `&SessionTitle`, and published through `AppEventHub` only after a successful database commit. The event contains only the Ora session id; clients refetch the authoritative `Session.title`. Title acquisition is never restored after process restart, agent switch, explicit stop, connection loss, or actor termination, and a late provider update cannot change a locked title.

Supervisors retry failed providers independently with capped backoff and reap the old process tree before replacement. Ora remains available when one or all providers are unavailable.

See the [ora-backend overview](../../README.md) and [ACP Agent Runtime design](../../../../docs/agent-runtime.md).
