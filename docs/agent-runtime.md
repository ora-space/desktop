# ACP Agent Runtime

`ora-backend` owns one serialized actor for every persisted Ora Session. An actor owns at most one ACP CLI child and accepts only one load or prompt operation at a time. Different Sessions remain concurrent.

## Process and Session Lifecycle

- `AgentCli` is immutable and currently supports `open_code`, `nga`, and `code_agent_cli`. The three executable-path functions are deliberately separate even though they currently share the same home-directory layout.
- Every child starts with the single `acp` argument and piped stdin, stdout, and stderr.
- The child cwd is resolved from Task → stored Worktree id → stored branch name → Git's authoritative worktree metadata. A configured worktree creation root is never used to reconstruct an existing path.
- Create performs `initialize`, then `session/new`, and persists the Ora Session only after both succeed. The guarded insert fails if its Task was deleted while the provider handshake was in flight.
- Load starts a fresh child, performs `initialize`, then calls `session/load` with the private `agentSessionId`. The row is reserved as Running before process setup so aggregate deletion cannot race the load; every setup or replay failure restores Stopped.
- Create drains setup notifications emitted before the `session/new` response and returns the latest complete available-command list so the first composer render has provider commands.
- Prompt accepts ordered ACP content blocks so supported providers can receive text and image content in one turn. The serialized public payload is bounded at 16 MiB.
- Startup reconciles stale Running database rows to Stopped because child ownership cannot survive an Ora process restart.

## Flow Control

ACP stdout is newline-delimited JSON-RPC with an 8 MiB frame limit. Session updates use a bounded 256-item data queue. Permissions and fatal transport conditions use an independent control queue, so a full data queue cannot suppress a permission decision. A full data queue is fatal; no data is silently discarded.

Unknown agent-originated JSON-RPC requests receive a correlated `-32601` method-not-found response and do not terminate the connection. Malformed frames, unmatched responses, oversized frames, and updates for another Session are protocol failures.

Dropping a Web body, closing a Tauri stream, or aborting the frontend `AsyncIterable` sends `session/cancel`. Prompt cancellation waits up to five seconds for the provider to settle and keeps the process only when the connection remains usable; otherwise the process tree is killed and the Session becomes Stopped. Explicit Stop always terminates the child while preserving provider history for a later load.

## Ownership Boundaries

Ora deletion removes only Ora-owned database records. It does not call ACP session delete and does not touch Git branches or worktrees. Session deletion serializes against new actor operations, stops an owned child, and then soft-deletes the row under the same lifecycle guard. Task and Project deletion reject Running descendants and transactionally cascade stopped Ora records.
