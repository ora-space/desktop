# ACP Agent Runtime

`ora-backend` starts one independently supervised ACP child for each supported CLI (`opencode`, `nga`, and `codeagentcli`) when a Backend instance opens. Every persisted Ora Session owns a serialized actor, but actors targeting the same CLI share its application-scoped ACP connection and route events by the private provider session id. One Session accepts only one load or prompt operation at a time while different Sessions remain concurrent.

## Process and Session Lifecycle

- Session contracts select an `agent_cli`; persistence stores the stable values `ora-space.opencode`, `ora-space.nga`, and `ora-space.codeagentcli` as text.
- The shared child starts in the user's home directory with the single `acp` argument and piped stdin, stdout, and stderr. Session setup requests carry the owning Task worktree as `cwd`.
- Task worktrees resolve through Task → stored Worktree id → stored branch name → Git's authoritative worktree metadata. A configured worktree creation root is never used to reconstruct an existing path.
- Backend startup reconciles stale Running rows to Stopped, then one dedicated runtime thread per CLI attempts startup and performs `initialize`. Owning the runtimes here is necessary because synchronous Desktop bootstrap does not guarantee an ambient Tokio runtime. Each CLI retries independently with capped exponential backoff; Ora remains available even if every initial attempt fails, and one unavailable CLI does not disable the others.
- Create calls `session/new` on the ready shared connection and persists the Ora Session only after setup succeeds. The guarded insert fails if its Task was deleted while the handshake was in flight.
- Load registers a route on the current connection generation, marks the row Running, and calls `session/load` with the private `agentSessionId`. Every setup or replay failure restores Stopped.
- Connection loss fails that CLI's in-flight operations, marks only its registered Sessions Stopped, terminates and reaps the old process tree, and only then starts a replacement. Sessions are loaded again only on demand; prompts are never replayed automatically.
- Model discovery runs each CLI's bounded `models` command concurrently. The response is grouped by `agent_cli` and omits CLIs whose command is missing, fails, emits invalid UTF-8, or exceeds the timeout, allowing partial results.
- Create returns the latest complete available-command list emitted during `session/new` setup so the first composer render has provider commands.
- Prompt accepts ordered ACP content blocks so supported providers can receive text and image content in one turn. The serialized public payload is bounded at 16 MiB.

## Flow Control

ACP stdout is newline-delimited JSON-RPC with an 8 MiB frame limit. The connection reader uses an unbounded handoff to the always-running central router, while each registered Session owns a bounded 256-item update queue and an independent control queue. This keeps connection-wide parsing from imposing one Session's backpressure on another. A per-Session overflow stops only the affected Session; no data is silently discarded.

Unknown agent-originated JSON-RPC requests receive a correlated `-32601` method-not-found response and do not terminate the connection. Malformed frames, unmatched responses, oversized frames, and stdio loss are connection failures. Updates for unloaded Sessions are treated as stale and discarded rather than taking down unrelated work, except during the bounded `session/new` setup window before the provider id is known.

Dropping a Web body, closing a Tauri stream, or aborting the frontend `AsyncIterable` sends `session/cancel`. Prompt cancellation waits up to five seconds for the provider to settle. A session-level timeout unloads and stops only that Session; it never restarts the shared process. Explicit Stop optionally calls `session/close` when advertised, unloads the route, and preserves provider history for a later load.

## Ownership Boundaries

Ora deletion removes only Ora-owned database records. It does not call ACP session delete and does not touch Git branches or worktrees. Session deletion serializes against new actor operations, unloads its route, and then soft-deletes the row under the same lifecycle guard. Task and Project deletion reject Running descendants and transactionally cascade stopped Ora records.

Dropping the last Backend owner asks every supervisor to stop accepting work, cancels routed operations, and initiates bounded termination and reaping of each CLI process tree. Successful processes remain alive while the Backend exists even when no Sessions are registered.
