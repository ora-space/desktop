# ACP Agent Runtime

`ora-backend` starts one independently supervised ACP child for each supported CLI (`opencode`, `nga`, and `codeagentcli`) when a Backend instance opens. Every persisted Ora Session owns a serialized actor, but actors targeting the same CLI share its application-scoped ACP connection and route events by the private provider session id. One Session accepts only one load or prompt operation at a time while different Sessions remain concurrent.

## Process and Session Lifecycle

- Session contracts select an `agent_cli`; persistence stores the stable values `ora-space.opencode`, `ora-space.nga`, and `ora-space.codeagentcli` as text, so the persisted value does not depend on enum declaration order.
- Executable resolution is platform-specific and repeated on every retry generation. On Unix each CLI is read from its fixed per-user directory — `<home>/.opencode/bin/opencode`, `<home>/.nga/bin/nga`, `<home>/.codeagentcli/bin/codeagentcli`. On Windows it is resolved from `PATH` through `where.exe`, preferring an `.exe`, `.cmd`, or `.bat` match. A CLI that cannot be resolved reports `agent_cli_not_found`.
- The shared child starts in the user's home directory with the single `acp` argument and piped stdin, stdout, and stderr. Stderr is drained continuously so provider diagnostics can never block the process. Session setup requests carry the owning Task worktree as `cwd`.
- Task worktrees resolve through Task → stored Worktree id → stored branch name → Git's authoritative worktree metadata. A configured worktree creation root is never used to reconstruct an existing path.
- Backend startup reconciles stale Running rows to Stopped, then one dedicated runtime thread per CLI attempts startup and performs `initialize`. Owning the runtimes here is necessary because synchronous Desktop bootstrap does not guarantee an ambient Tokio runtime. Each CLI retries independently with capped exponential backoff; Ora remains available even if every initial attempt fails, and one unavailable CLI does not disable the others. Operations targeting an unavailable CLI report `agent_runtime_unavailable`.
- Warm creates open a temporary setup-registration window and call `session/new` on the ready shared connection, but persist nothing. Because the provider session id is not known until the response arrives, otherwise-unrouted setup notifications are buffered during that window and transferred to the matching session route once its id is known. The latest setup-time `available_commands_update` is kept with the warm session, because nothing consumes its updates until it is attached and the announcement would otherwise be lost.
- Attach binds one warm session to its owning Task and persists the Ora Session under the identifier the client already holds, reporting the captured catalog as `AttachSessionResponse.availableCommands`. The guarded insert fails if its Task was deleted while the handshake was in flight. The session's history file is opened with its header record before the session can be prompted.
- Load registers a route on the current connection generation, marks the row Running, and calls `session/load` with the private `agentSessionId`. Every setup or replay failure restores Stopped. The agent's replay is drained and discarded — `session/load` is called so the agent restores the context it needs to answer the next prompt, not to tell Ora what the conversation was. What the client receives is Ora's own record. See [Session History](#session-history).
- Connection loss fails that CLI's in-flight operations, marks only its registered Sessions Stopped, terminates and reaps the old process tree, and only then starts a replacement. Sessions are loaded again only on demand; prompts are never replayed automatically.
- The `initialize` handshake advertises the client's session config-option capability. Agents withhold configuration options from clients that do not, so the model selector depends on it. Boolean options stay undeclared because Ora renders only id-valued selectors.

## Warm Sessions

ACP reports a session's configuration options — the model selector among them — only in the reply to `session/new` or `session/load`. A model therefore cannot be chosen until a session exists, so opening a chat surface creates one ahead of the first prompt.

- Warm sessions live only in memory. No Ora Session row exists until attach, so abandoning a chat leaves no empty history behind on the Ora side.
- Each warm session is keyed by `(target, agent_cli, client_id)`, where target is a Task or a project root. The key includes the client because one backend serves several clients; without it two browser tabs on the same selection would share a session and the first attach would take the other tab's conversation.
- The Ora session identifier is minted when the session is warmed, not when it is persisted, so the client never has to follow an identifier change. Before attach that identifier is absent from `getSession` and `listSessions`.
- The working directory is re-derived from the target on every request and compared with the one the session was created against. A worktree that moved or was recreated retires the session instead of quietly addressing a stale path.
- Configuration changes apply to warm and persisted sessions alike and are remembered. A superseded connection generation or the live-session bound releases the provider session while the entry survives as a cold record of the identifier, its directory, and the options the user chose.
- Nothing releases a warm session for having sat unused. A warm session is one the user never prompted, so an idle entry holds no conversation to reclaim; the live-session bound alone caps how many exist, and it is applied when another session is created. A chat left open and returned to is still live rather than rebuilt.
- Deleting a Task or project discards the warm sessions for its chat surfaces and returns their provider sessions. This is the only thing that reclaims them: the target is gone, so no request can name the entry again, and the live-session bound displaces it only once enough new surfaces are opened. A project delete cascades to its Tasks, so their surfaces are discarded alongside the project root's. A warm session being attached is left alone — that attach owns its provider session now.
- A cold entry is rebuilt transparently on the next use, replaying those options. A replay the agent rejects degrades to whatever the agent reports rather than failing a prompt the user already typed, and the corrected options reach the client as a `session/update`.
- Attach rebuilds rather than reuses when the Task's directory differs from the warm session's, which is what makes it safe to warm a direct chat against its project root before its Task exists.

## Session History

Ora records every conversation itself, in one append-only JSONL file per Session under the configured sessions root. This is what lets a conversation outlive one provider: it is replayed without asking the agent to recite it, and it can be handed to a different agent entirely. The file format, its ordering rules, and its failure semantics belong to [`ora-history`](../crates/history/README.md); this section covers only when the runtime uses them.

- The runtime records what it chose to keep, not what it sent. A prompt is recorded from the request blocks before the agent is called, and the provider's echoed `user_message_chunk` is ignored, so context Ora injected never enters the record.
- Streamed updates are recorded before they are forwarded. A client that disconnects mid-turn costs the stream, never the record of what the agent produced.
- Every prompt turn closes with its `stopReason`. Provider replay never carried this, so a cancelled turn used to be indistinguishable from a completed one; replaying Ora's record restores it, along with the tool calls that never finished.
- Ordered session events make the response a turn fence. Updates and permission requests observed before it are consumed by the same operation, and the turn is recorded only after that fence is settled. Cancellation keeps consuming through the matching response during its grace period; if the provider does not settle, the actor records the accepted event snapshot before isolating the route.
- Writes are batched per settled item, flushed but not synced. A crash costs at most the item in flight.

### Switching Agents

`switchSessionAgent` moves one conversation onto a different CLI. The session keeps its identifier, its Task, and its history; only the binding changes.

- The new provider session is created **before** anything is torn down, so an unavailable CLI or a failed handshake leaves the conversation exactly where it was. Switching to the CLI a session already runs on reports `session_agent_unchanged`; a session whose history is degraded reports `session_history_degraded`.
- A failure *after* that handshake closes the new provider session before returning. Nothing else would: no Ora row names it yet, and dropping its channel unregisters routing without telling the CLI.
- Once the move is certain the old binding is released. It is not kept for a later switch back: its context stops at the moment it was left, so returning to it would need the intervening turns anyway — and injecting a full transcript into a fresh session is simpler and more predictable than reconciling a stale one.
- The transcript is injected **lazily**. Switching sends nothing; the recorded conversation is prepended to the next prompt as a leading content block. A session switched and then abandoned costs nothing.
- Whether the current binding still needs the transcript is derived from the record — a trailing `AgentSwitched` with no user message after it — so it survives a restart without any stored flag. Because that is also what recording a prompt undoes, a history Ora cannot read at injection time defers the transcript to the next prompt instead of dropping it: the question would never be asked again.
- ACP offers no way to install a conversation into an agent: `session/new` takes no context and `session/prompt` takes one user turn. Every recorded turn therefore collapses into a single user message that the receiving agent is *shown* rather than one it took part in, which is what the injected block's preamble exists to explain.
- There is no size budget on the transcript. A long conversation can exceed the receiving model's context window, and that failure surfaces from the provider.

### Degraded History

A history that skips records is more dangerous than one that stops, because the gap is invisible to whoever replays it — including the next agent. So a failed write stops recording that session for good rather than continuing past the hole.

- A turn already streaming finishes: the agent's work is real whether or not the file kept it, and failing it would tell the user nothing happened when something did.
- A turn whose own prompt could not be recorded is refused before the agent is called. Nothing has happened yet, so nothing is lost by refusing it, and sending it would move the conversation somewhere the record cannot follow. If that prompt was the one carrying a handoff transcript, the binding still owes it and the next prompt carries it instead.
- The session moves to `historyState: degraded` carrying the operating-system reason, and further prompts are refused with `session_history_degraded` until it is resumed.
- `resumeSessionHistory` appends a `Gap` record naming what interrupted the file *before* accepting new content, then returns the session to writable. Resuming does not restore what was lost; it records that something was.
- A history file Ora cannot read degrades the session the same way. Appending without knowing which positions are already used would overwrite them.
- A load whose history cannot be read fails the load rather than completing an empty one. Load is how a user asks to see the conversation, and an empty view is indistinguishable from a session that never said anything.

### Deletion

Ora's soft delete is what a user experiences as deletion, so the conversation goes with it: deleting a Session removes its history file, and Task and Project cascades remove the files of every session they take with them. The session identifiers are collected before the cascade, because afterwards nothing links the files back to the task that owned them. Removal is best effort — the rows are already gone, an orphaned file is unreachable, and failing here would leave the user with something they cannot delete.

## Flow Control

ACP stdout is newline-delimited JSON-RPC with an 8 MiB frame limit. The connection reader uses an unbounded, ordered handoff to the always-running central router, while each registered Session owns a bounded 256-item FIFO for updates, permission requests, and terminating responses. Connection loss and queue overflow use an independent control queue. This keeps connection-wide parsing from imposing one Session's backpressure on another while preserving the event order needed to close a turn safely. A per-Session overflow stops only the affected Session; an active operation drains events already accepted before applying that terminal control.

Session setup has a separate bounded buffer for notifications that arrive before `session/new` reveals their provider session id. It is active only while one or more creates are in flight, holds at most 256 notifications across those setups, and retains the newest notifications when full. Registering a session drains only notifications whose provider id matches that route; when the final setup window closes, any still-unmatched notifications are discarded as stale.

Unknown agent-originated JSON-RPC requests receive a correlated `-32601` method-not-found response and do not terminate the connection. Malformed frames, unmatched responses, oversized frames, and stdio loss are connection failures. Routes are generation-bound, so updates from an old connection or an unloaded Session are treated as stale and discarded rather than taking down unrelated work.

Permission requests are part of the ordered session FIFO, so a prompt consumes them in the same order in which the provider emitted them and can correlate the user's response to the active operation. A permission request arriving during `session/load` or while the session is idle is answered as cancelled; the operation reports the backend's typed internal error when protocol traffic violates that lifecycle boundary. Connection loss and queue overflow remain separate terminal controls.

Dropping a Web body, closing a Tauri stream, or aborting the frontend `AsyncIterable` sends `session/cancel`. A session-level timeout unloads and stops only that Session; it never restarts the shared process. Explicit Stop optionally calls `session/close` when advertised, unloads the route, and preserves provider history for a later load.

History replay is the one stream that applies backpressure instead of failing fast. A recorded conversation is far larger than the 256-item queue, and a consumer that has not drained it yet is not a disconnected one.

## Timeouts and Limits

| Bound | Value |
| --- | --- |
| `initialize` handshake | 15 s |
| Load and prompt inactivity deadline | 30 s, reset by each session update |
| Cancellation settlement grace | 5 s |
| Connection retry backoff | 250 ms, doubling to a 30 s cap |
| Session update and event queue depth | 256 items |
| JSON-RPC frame size | 8 MiB |
| Serialized structured prompt size | 16 MiB |
| Handoff transcript size | unbounded |

The load and prompt deadline is an inactivity timer rather than a total budget: a provider that keeps streaming updates can run indefinitely, while one that goes silent for 30 seconds fails that Session alone. Prompts are passed through as ordered ACP `ContentBlock` values, including text, images, audio, resource links, and embedded resources. An empty list or a list containing only blank text is rejected, and the 16 MiB limit is measured from the serialized JSON payload before it reaches the provider.

## Ownership Boundaries

Ora deletion removes Ora-owned database records and the session history Ora itself wrote. It does not touch Git branches or worktrees, so provider-side history survives a deletion on the Ora side. ACP session delete is called in exactly one case: retiring a warm session that Ora created, never persisted, and never handed to the user. Those sessions carry no history and no Ora record, and deleting them is what stops abandoned chat surfaces from accumulating empty sessions inside providers that persist them. Every session the user can see is closed, never deleted. Session deletion serializes against new actor operations, unloads its route, soft-deletes the row under the same lifecycle guard, and then removes the history file. Task and Project deletion reject Running descendants and transactionally cascade stopped Ora records.

Ora owns the transcript; the agent owns the model context. That split is what the whole design turns on: the transcript is portable between agents and the context is not, which is why a switch replays nothing and injects instead.

Dropping the last Backend owner asks every supervisor to stop accepting work, cancels routed operations, and initiates bounded termination and reaping of each CLI process tree. Successful processes remain alive while the Backend exists even when no Sessions are registered.
