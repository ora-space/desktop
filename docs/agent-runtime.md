# ACP Agent Runtime

`ora-backend` starts one independently supervised ACP connection for each installed [agent plugin](../crates/backend/src/agent_runtime/plugin_agent/README.md) when a Backend instance opens — every agent Ora can reach is supplied by a plugin package, and there is no other source. Every persisted Ora Session owns a serialized actor, but actors targeting the same agent share its application-scoped ACP connection and route events by the private provider session id. One Session accepts only one prompt owner at a time, while load streams may follow that prompt and different Sessions remain concurrent. Session-scoped prompt cancellation reaches that owner without granting a load stream ownership or unloading the reusable Session.

## Process and Session Lifecycle

- Session contracts select an `agent_ref`: the canonical `<namespace>/<name>` id of the installed plugin that supplies the agent, carried as an open string. The namespace is part of the identity, not decoration — a marketplace source's namespace decides it, and two sources may publish packages under the same `identifier`. Dropping it would collapse those two packages onto one agent, so whichever was walked first would serve both packages' sessions. Which agents exist depends entirely on installed plugins and is not knowable when Ora is built, so an identity the runtime does not recognize means "that provider is not installed right now" — an ordinary runtime state reported as `agent_runtime_unavailable`, not corrupt data. Nothing is bundled: Ora ships with zero agents until the user installs plugin packages that supply them, one agent per package (see [plugin_agent](../crates/backend/src/agent_runtime/plugin_agent/README.md)).
- Supervisors are keyed by that identity, and because it is the whole plugin id, no two installed packages can claim one: there is no arbitration, and the supervised set is exactly the installed agent set. Only `agent`-kind packages supply an agent; `ui` packages contribute surfaces and are never supervised. The installed set comes from the plugin lifecycle, which is also the sole owner of plugin processes: a supervisor attaches to a running plugin (`PluginApi::attach_agent`, which starts an installed plugin on demand) instead of launching one, reads that generation's `agent/acp` notifications through a lossless tap, and asks the lifecycle to stop the plugin when a generation ends.
- The supervised set is reconciled whenever installed packages change. Installing a plugin makes its agent reachable in the running process, uninstalling it removes the supervisor, and every installed agent plugin can always start.
- The plugin owns the agent process end to end: it decides which CLI to run and relays ACP frames to the host over its `agent/acp` notification channel. The host never chooses a CLI or interprets what it says. It does own the OS process, and it resolves the executable path for a plugin that ships its own binary: `ora/childprocess/spawn` accepts either a `command` the operating system resolves or a `packageCommand` the host joins onto that plugin's install root, because a plugin is told no host path and cannot reliably compute one.
- Both ways of reaching a CLI stay open, and the plugin picks per spawn. An agent package is published either with a bundled CLI (a `[[targets]]` release, one package per target triple, whose in-package `orax.toml` declares `[artifact]`) or without one (a universal `url`/`sha256` release that resolves the user's own install from PATH). The same plugin source serves both, because it cannot know at build time which package it ended up in: it asks for its bundled path first, and the host's `package_command_missing` answer — distinct from every other package-resolution failure — is what tells it to fall back to a PATH lookup.
- The two ways that fails are classified apart, because the user can only fix one of them without restarting Ora. A CLI absent from this machine reports `agent_not_installed` (`-32001`), stays retryable, and is not logged per attempt, so installing it later is enough to bring the agent up. A package whose bundled executable will not resolve or run reports `agent_unusable` (`-32002`): it fails identically on every attempt, so that agent is abandoned for the rest of the process instead of being retried quietly behind a missing-CLI report. Only that second report carries the plugin's own message through, because giving up makes it the only diagnostic the failure ever produces and only the plugin knows which of its programs failed.
- Task worktrees resolve through Task → Workspace id → stored Worktree branch name → Git's authoritative worktree metadata. A configured worktree creation root is never used to reconstruct an existing path. Main project workspaces resolve their stored location against the bootstrap path base (the parent of `ORA_DATA_DIR` in Desktop), not live process cwd.
- Backend startup reconciles stale Running rows to Stopped, then one dedicated runtime thread per agent attempts startup and performs `initialize`. Owning the runtimes here is necessary because synchronous Desktop bootstrap does not guarantee an ambient Tokio runtime. Each agent retries independently with capped exponential backoff; Ora remains available even if every initial attempt fails, and one unavailable agent does not disable the others. Operations targeting an unavailable agent report `agent_runtime_unavailable`. A missing agent process remains retryable and does not count as a crash. More than three genuine startup or connection failures in one minute publishes `Failing` and stops that agent's automatic retries for the rest of the process. A plugin whose registration does not implement the agent contract enters `Failing` immediately because every retry would repeat the same deterministic failure.
- `startSession` is the sole creation path. It opens a temporary setup-registration window, calls `session/new` with the current Session MCP Snapshot, applies the caller's optional model intent against the configuration reported by that handshake, persists the Ora Session, opens its history, and installs its actor before returning. The route channel opened for setup is handed directly to the actor, so non-command setup notifications are not lost.
- A provider session created by `startSession` remains guarded until persistence and actor registration finish. Any error or cancelled command future before that point releases the unexposed provider session with `session/delete` when supported and `session/close` otherwise; failed persistence also removes any history file and soft-deletes a row that was already inserted.
- Load registers a route on the current connection generation when the provider conversation is idle, marks the row Running, and calls `session/load` with the private `agentSessionId` and the current Session MCP Snapshot. The caller's admission signal resolves only after the Running row is persisted (and the row's task and project were re-validated as visible), so an aggregate deletion that starts after admission is guaranteed to observe the Running session and refuse, while a deletion that already committed makes the admission itself fail. Every setup or replay failure restores Stopped. The agent's replay is drained and discarded — `session/load` is called so the agent restores the context it needs to answer the next prompt, not to tell Ora what the conversation was. What the client receives is Ora's own record, including typed notices for known holes rather than a transcript that silently appears continuous. If a prompt already owns the session, load replays the record through its attachment point and then follows later events from that prompt; it neither starts a second prompt nor takes ownership of the first. See [Session History](#session-history) and [Session MCP](session-mcp.md).

## Session MCP

Configured MCP plugins are Session Runtime Input. `startSession` `session/new`, explicit `session/load`, live refresh, workflow start, and agent replacement all resolve one Snapshot and send it as ACP `mcpServers`. Live Sessions keep Desired/Active revisions in memory; a configuration change wakes idle Sessions immediately and busy Sessions after the current prompt. Refresh and Skill Effect mutation share one Agent Session Barrier. Setting values never enter Effect state, SQLite, Workspace files, logs, or UI. See [Session MCP](session-mcp.md).

- Connection loss fails that agent's in-flight operations, marks only its registered Sessions Stopped, asks the plugin lifecycle to stop the old process, and only then starts a replacement. Sessions are loaded again only on demand; prompts are never replayed automatically.
- The `initialize` handshake advertises the client's session config-option capability. Agents withhold configuration options from clients that do not, so the model selector depends on it. Boolean options stay undeclared because Ora renders only id-valued selectors.

### First session title acquisition

A newly attached Session owns a runtime-only title window. It accepts valid `session_info_update` titles immediately, even before the first prompt completes. The first prompt only starts the fallback window when it ends with `EndTurn`, `MaxTokens`, or `MaxTurnRequests`; refusal, cancellation, request failure, and connection failure do not consume that eligibility.

- If initialization advertises `session/list`, the actor schedules attempts three and ten seconds after the eligible prompt. Each request is sent only while the actor is idle, uses the provider session id to select the matching entry, and has a five-second timeout.
- If `session/list` is not advertised, no list request is sent; the actor remains open to push titles until the ten-second boundary, then locks.
- Push and list titles share the same `SessionTitle` validation and persistence path. While the window is open, the last valid agent title whose database write succeeds wins. A duplicate title does not write or publish an event; blank, null, and overlong values are ignored. A user-driven rename locks the window so a later agent title cannot overwrite it.
- Scheduler callbacks enqueue actor commands only. A prompt, load, stop, switch, delete, user rename, queue failure, connection loss, or actor termination closes or preempts the window, so title polling never delays user work. A `TitleUpdate` received while a list request is in flight is handled and the request continues; a late `Cancel` belonging to a completed prompt is ignored. The acquisition state is not stored in SQLite and is disabled when an existing Session is restored. A user-driven rename locks the window (and re-persists the chosen title) so a later push or list title cannot overwrite it.
- After a successful title-only database update, the actor publishes `AppEvent::SessionTitleUpdated { session_id }`. The event is an invalidation hint, not a log; the frontend refetches the authoritative `Session.title`.

Actor-owned scheduler callbacks keep only a weak command sender. Once deletion, switching, or manager shutdown releases the last external sender, the actor command receiver closes and the actor releases its connection, recorder, repository, and scheduler clones. This is also why scheduler callbacks upgrade their sender at execution time rather than keeping a hidden actor-owning clone.

## Lazy Session Creation and Model Discovery

Opening or navigating to a chat surface creates no backend session state. The frontend keeps the optimistic first turn locally while `startSession` performs the provider handshake and persistence, then adopts the returned Ora session id before prompting it. Workflow nodes use the same path with an unpublished marker until the node-run binding commits.

Pre-session models come from `agent/list_models`, called on demand with the Workspace's resolved working directory. Ora does not cache the result or read it while bringing up the shared ACP connection. The plugin owns discovery and its cache lifetime; if discovery requires ACP, the plugin must ask the host's child-process service to launch a separate one-shot agent process rather than injecting frames into Ora's shared connection or spawning an unmanaged process itself. The call has a dedicated 60-second budget, and a failure affects only that discovery request, never the shared connection lifecycle. Empty model lists are valid.

The frontend query cache controls repeated discovery. A model selected before a session exists is local intent sent with `startSession` or `switchSessionAgent`; the backend applies it only when the newly-created session reports the exact value. The response always carries that session's authoritative configuration options.

## Session History

Ora records every conversation itself, in one append-only JSONL file per Session under the configured sessions root. This is what lets a conversation outlive one provider: it is replayed without asking the agent to recite it, and it can be handed to a different agent entirely. The file format, its ordering rules, and its failure semantics belong to [`ora-history`](../crates/history/README.md); this section covers only when the runtime uses them.

- The runtime records what it chose to keep, not what it sent. A prompt is recorded from the request blocks before the agent is called, and the provider's echoed `user_message_chunk` is ignored, so context Ora injected never enters the record.
- Streamed updates are recorded before they are forwarded. A client that disconnects mid-turn costs the stream, never the record of what the agent produced.
- Every prompt turn closes with its `stopReason`. Provider replay never carried this, so a cancelled turn used to be indistinguishable from a completed one; replaying Ora's record restores it, along with the tool calls that never finished.
- The turn boundary settles tool calls the agent left open. ACP does not require an agent to report a terminal status, and an agent that opens a call and moves on would otherwise record work that appears to still be running forever. Only a turn the agent ended on its own terms records those calls as completed. Every other ending — cancelled, out of tokens, refused — leaves them unfinished, because the call may have been interrupted rather than finished and `stopReason` already says which. Since the runtime records a lost connection, a failed prompt, an overflowing queue, and a disconnected client all as cancelled, a tool interrupted by any of them keeps its unfinished status rather than being credited with a result nobody saw. Readers settle what the record leaves unfinished: the conversation view shows a cut-short turn's open calls as interrupted, and the handoff transcript names their outcome as unreported rather than claiming the tool never ran.
- Ordered session events make the response a turn fence. Updates and permission requests observed before it are consumed by the same operation, and the turn is recorded only after that fence is settled. Cancellation keeps consuming through the matching response during its grace period; if the provider does not settle, the actor records the accepted event snapshot before isolating the route.
- Writes are batched per settled item, flushed but not synced. A crash costs at most the item in flight.
- A complete JSONL line that cannot be decoded is omitted from replay but reported as an `unreadable_records` history notice with the aggregate count. Its turn position is unknown because append order is not timeline order, so neither the backend nor client guesses one. The conversation remains writable: past corruption is an integrity warning, while `historyState` continues to describe whether new records can be appended. Existing durable `Gap` records are replayed as `unrecorded_content` notices instead of being skipped. The chat store stages notices with the rest of a load, the UI keeps a non-blocking warning above the conversation, and an agent handoff labels the surviving transcript incomplete.

### Switching Agents

`switchSessionAgent` moves one conversation onto a different CLI. The session keeps its identifier, its Task, and its history; only the binding changes.

- The incoming provider session is handshaken and optionally configured before the lifecycle lock is taken. Nothing is torn down until that succeeds, so an unavailable CLI leaves the conversation exactly where it was. Switching to the currently bound CLI reports `session_agent_unchanged`; degraded history reports `session_history_degraded`.
- Because rebinding stops the current actor, the client commits the move with the next message rather than when the agent is picked. Agent and model selections remain local until that send, and returning to the currently bound agent withdraws the pending move.
- Once the move is certain the old binding is released. It is not kept for a later switch back: its context stops at the moment it was left, so returning to it would need the intervening turns anyway — and injecting a full transcript into a fresh session is simpler and more predictable than reconciling a stale one.
- The transcript is injected **lazily**. Switching sends nothing; the recorded conversation is prepended to the next prompt as a leading content block. A session switched and then abandoned costs nothing.
- The response carries the new handshake's `availableCommands` and `configOptions`, so the client immediately replaces the outgoing agent's capabilities with authoritative state from the incoming session.
- Whether the current binding still needs the transcript is derived from the record — a trailing `AgentSwitched` with no `HandoffDelivered` after it — so it survives losing the actor that tracked it, and a restart, without any stored flag.
- The debt is settled only once the provider has **accepted** the prompt carrying the transcript, in memory and in the file together. Everything before that point can still fail: a history Ora cannot read at injection time, a prompt refused because its own write failed, and a `session/prompt` that never left the process all leave the binding still owing its transcript, and the next prompt carries it instead. A user turn cannot stand in for the delivery record, because Ora records a prompt _before_ sending it — one sitting after a switch is equally consistent with a delivered transcript and with a request that failed on the way out.
- Accepting the request is the last thing Ora can observe about delivery; past it the frame is on the agent's stdin. A connection lost mid-turn therefore counts as delivered rather than re-injecting the whole conversation into an agent that already holds it — the transcript's preamble, which tells its reader the work belongs to a _different_ agent, would be false if it did. Failing to record the delivery goes the other way and hands the transcript over again, which is the harmless direction to be wrong in.
- ACP offers no way to install a conversation into an agent: `session/new` takes no context and `session/prompt` takes one user turn. Every recorded turn therefore collapses into a single user message that the receiving agent is _shown_ rather than one it took part in, which is what the injected block's preamble exists to explain.
- There is no size budget on the transcript. A long conversation can exceed the receiving model's context window, and that failure surfaces from the provider.

### Degraded History

A history that skips records is more dangerous than one that stops, because the gap is invisible to whoever replays it — including the next agent. So a failed write stops recording that session for good rather than continuing past the hole.

- A turn already streaming finishes: the agent's work is real whether or not the file kept it, and failing it would tell the user nothing happened when something did.
- A turn whose own prompt could not be recorded is refused before the agent is called. Nothing has happened yet, so nothing is lost by refusing it, and sending it would move the conversation somewhere the record cannot follow. If that prompt was the one carrying a handoff transcript, the binding still owes it — no delivery was recorded — and the next prompt carries it instead.
- The session moves to `historyState: degraded` carrying the operating-system reason, and further prompts are refused with `session_history_degraded` until it is resumed.
- `resumeSessionHistory` appends a `Gap` record naming what interrupted the file _before_ accepting new content, then returns the session to writable. Resuming does not restore what was lost; it records that something was.
- A history file Ora cannot read at all degrades the session the same way. Appending without knowing which positions are already used would overwrite them. This is distinct from individual complete lines that are unreadable: those preserve the surviving timeline and surface an integrity notice as described above.
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

| Bound                                | Value                                        |
| ------------------------------------ | -------------------------------------------- |
| `initialize` handshake               | 15 s                                         |
| Plugin-owned model discovery         | 60 s                                         |
| Load and prompt inactivity deadline  | 30 s, reset by each session update           |
| Cancellation settlement grace        | 5 s                                          |
| Connection retry backoff             | 250 ms, doubling to a 30 s cap               |
| Connection crash circuit             | Opens after more than 3 failures in 1 minute |
| Session-list title request           | 5 s per attempt                              |
| First-title fallback window          | 3 s and 10 s after the first eligible prompt |
| Session update and event queue depth | 256 items                                    |
| JSON-RPC frame size                  | 8 MiB                                        |
| Serialized structured prompt size    | 16 MiB                                       |
| Handoff transcript size              | unbounded                                    |

The load and prompt deadline is an inactivity timer rather than a total budget: a provider that keeps streaming updates can run indefinitely, while one that goes silent for 30 seconds fails that Session alone. Prompts are passed through as ordered ACP `ContentBlock` values, including text, images, audio, resource links, and embedded resources. An empty list or a list containing only blank text is rejected, and the 16 MiB limit is measured from the serialized JSON payload before it reaches the provider.

## Ownership Boundaries

Ora deletion removes Ora-owned database records and the session history Ora itself wrote, and registers durable Git cleanup jobs (executed asynchronously by the backend worker) for the deleted tasks' worktrees and `ora/*` branches. Provider-side history survives deletion on the Ora side. ACP `session/delete` is used only to roll back a provider session that failed before becoming visible; every user-visible session is closed, never deleted. Session deletion serializes against new actor operations, unloads its route, soft-deletes the row under the same lifecycle guard, and then removes the history file. Task and Project deletion reject Running descendants and transactionally cascade stopped Ora records.

Ora owns the transcript; the agent owns the model context. That split is what the whole design turns on: the transcript is portable between agents and the context is not, which is why a switch replays nothing and injects instead.

Dropping the last Backend owner asks every supervisor to stop accepting work, cancels routed operations, and initiates bounded termination and reaping of each CLI process tree. Successful processes remain alive while the Backend exists even when no Sessions are registered.

## Caveats After Opening the Agent Identity

Agent identity is an open string: agents arrive with installed plugins, and which ones exist is not
knowable when Ora is built. It is now the package's full canonical plugin id — `official/ora-space.claude`,
not `ora-space.claude` — because the namespace is what keeps two sources' same-named packages apart.
Anything that stored a shorter spelling names no supervised agent:

- **Sessions and workflow graphs may still name an agent by an older spelling.** `sessions.agent_cli`
  and a saved graph's `executor.agentCli` are carried to the runtime exactly as written, so a value
  stored before agent identity carried its namespace matches no supervised agent and reports an
  unavailable provider until the agent is picked again. Nothing rewrites either: translating old
  spellings would be a compatibility layer, and rewriting snapshot JSON is a database migration that
  should be decided on deliberately rather than smuggled in. Re-picking the agent fixes it
  permanently.

The frontend agent catalog is entirely runtime-derived — the chat and workflow pickers list
whatever `listInstalled` reports as `kind: "agent"`, labelled and drawn with each package's own
`agentDisplayName` and `logo`, offering only the identities `getAgentRuntimeStatus` reports
`Ready` or `Starting` for. That one status answer already covers every way an agent can be out of
reach — a package that was never installed and so has no supervisor, and an agent process missing
on this machine — so the client models none of them separately.
`Unavailable` is polled on at a slow cadence because it is expected configuration that can resolve
itself; `Failing` is not, because the restart circuit is open for the rest of the process. A
stored preference naming an agent that is no longer reachable is carried forward unexamined rather
than dropped — the frontend does not validate agent identities against a closed set — and a
session's own binding is always reported as written, since that is what the conversation genuinely
runs on.
