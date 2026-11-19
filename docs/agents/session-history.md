# Session History

English | [中文](session-history.zh.md)

Ora records every conversation itself, in one append-only JSONL file per Session under
`<data root>/sessions/`. This is what lets a conversation outlive one provider: it is replayed
without asking the agent to recite it, and it can be handed to a different agent. The file format,
ordering rules and failure semantics belong to [`ora-history`](../../crates/history/README.md); this
document covers how the [agent runtime](agent-runtime.md) uses them.

Ora owns the transcript; the agent owns the model context. The transcript is portable between agents
and the context is not, which is why a switch replays nothing and injects instead.

## Recording

### On-demand binding diagnostics

The conversation's **Session information / diagnostics** action combines the Ora session id,
provider binding timeline and existing runtime observations. The session menu opens the same
panel. Workflow run pages additionally expose an execution index, retaining separate node,
scope, iteration and failed-attempt references even when they share an Ora session.

The session-owned `getSessionBindings` query reads existing `Meta` and `AgentSwitched` records
in semantic sequence order, preserves repeated bindings and ignores `HandoffDelivered` as a
binding boundary. The current database row marks only the matching last segment, or supplies a
separate live segment with unknown recording time. History timestamps are recording times,
not provider creation times or recent live activity. Missing, incomplete and unsupported histories
remain visible alongside known current bindings. Reading never starts or loads a provider.

Queries accept at most 32 distinct sessions, read files serially within a batch with an 8 MiB
per-file limit, and recheck the session row after IO. A changed row permits one retry; persistent
change returns a retry result. The frontend deduplicates reads without merging execution rows,
cancels scoped requests on close/switch and keeps failed batches separate from successful ones.
Provider ids are copied in full and remain outside public `Session` payloads, message bubbles and
session lists. Existing child details stay in the parent transcript; the panel can locate its
child tree by turn and group, mounting windowed rows without copying their contents into another view.

Provider attribution is retained inside the existing ACP updates: complete text/thought chunks keep
their metadata and annotations, and tool metadata replaces only the top-level namespaces present in
an increment (including explicit null). Text with different metadata cannot merge even when message
IDs collide or are absent; plans keep the latest snapshot per valid child session id, with
unattributed plans sharing the parent stream. A valid child source requires nonempty id and parentID
strings and a positive integer depth; explicit null or invalid sources follow parent-stream rules. A parent turn
ending does not imply that an explicitly projected OpenCode child tool completed. The JSONL schema
and paths are unchanged; older history with missing attribution remains readable but cannot recover
information that was never recorded.

- The runtime records what it chose to keep, not what it sent. A prompt is recorded from the request
  blocks before the agent is called, and the provider's echoed `user_message_chunk` is ignored, so
  context Ora injected never enters the record.
- Streamed updates are recorded before they are forwarded, so a client that disconnects mid-turn
  loses the stream, never the record.
- Every prompt turn closes with its `stopReason`, which also restores the tool calls that never
  finished.
- The response is a turn fence: updates and permission requests observed before it belong to the
  same operation, and the turn is recorded only after the fence settles. During cancellation the
  actor keeps consuming until the matching response within the grace period; if the provider does
  not settle, it records what it accepted before isolating the route.
- Each settled item is appended and flushed (not synced) and handed to the host's
  `RuntimeEvents::record_settled` before the next line is written, so a crash costs at most the item
  in flight and a mirroring host sees every line exactly once, in order.

### Tool and turn timing

Tool timing follows the ACP lifecycle, not receipt time. The first `pending` or `in_progress` update
anchors `startedAt`; `completed`, `failed` or the turn boundary freezes `durationMs` on a monotonic
clock. A tool that arrives already terminal has no timing because Ora never saw it start. Timing is
persisted only with the final tool snapshot, so older history stays readable and missing timing is
never invented.

Turn duration is reconstructed from the recorded user message and the matching `TurnEnded`. The
renderer receives anchors and final durations and repaints visible labels from one local one-second
clock, without polling the backend.

### Unfinished tool calls

After an automatic resend, history records a `PromptRetried` boundary within the same user turn.
Tool snapshots retain their provider-reported status across this boundary and at the end of the
retried turn. A late delta updates the original tool opening until the same ID is reopened.
After reopening, ACP deltas without attempt attribution update the new opening. If the same
ID was pending or in progress earlier in the turn before the final retry boundary, a
completion on the new opening does not count as a result for the final attempt.

ACP does not require agents to report a terminal tool status. For a turn without a resend, only a
normal end records unfinished parent calls as completed; every other ending (cancelled, out of tokens,
refused, or a lost connection, failed prompt, overflowing queue or disconnected client, all recorded
as cancelled) leaves them unfinished, because they may have been interrupted. The conversation view
shows such calls as interrupted, and a handoff transcript names their outcome as unreported.

### Integrity notices

A complete JSONL line that cannot be decoded is omitted from replay and reported as an
`unreadable_records` notice with the count; its turn position is not guessed. The conversation stays
writable. Recorded `Gap` entries replay as `unrecorded_content` notices. The UI keeps a
non-blocking warning above the conversation, and a handoff labels the transcript incomplete.

## Switching agents

`switchSessionAgent` moves a conversation onto another agent. The Session keeps its id, Workspace
and history; only the binding changes.

- The incoming provider session is handshaken (and optionally configured) before anything is torn
  down, so an unavailable agent leaves the conversation where it was. Switching to the current agent
  reports `session_agent_unchanged`; degraded history reports `session_history_degraded`.
- The client commits the switch with the next message rather than when the agent is picked, and
  returning to the current agent withdraws a pending switch.
- The old binding is released, not kept for a later switch back: its context stops at the moment it
  was left, and injecting a full transcript into a fresh session is simpler than reconciling a stale
  one.
- The transcript is injected lazily: the recorded conversation is prepended to the next prompt as a
  leading content block. ACP has no way to install a conversation, so every recorded turn collapses
  into one user message the receiving agent is shown, with a preamble explaining that.
- Whether the binding still owes its transcript is derived from the record (a trailing
  `AgentSwitched` with no `HandoffDelivered` after it), so it survives losing the actor and a
  restart.
- The debt is settled only when the provider accepts the prompt carrying the transcript. Anything
  earlier can still fail and leaves the debt for the next prompt. A connection lost after acceptance
  counts as delivered; failing to record delivery hands the transcript over again, which is the
  harmless direction to be wrong in.
- The response carries the new handshake's `availableCommands` and `configOptions`.
- The transcript has no size budget; a conversation larger than the receiving model's context fails
  at the provider.

A provider session Ora rebuilds because the agent could not restore it (see
[Agent runtime](agent-runtime.md#session-lifecycle)) goes through the same injection. The rebuilt
binding stays out of the row until the prompt carrying the transcript is accepted.

## Degraded history

A history that skips records is more dangerous than one that stops, because the gap is invisible to
whoever replays it. A failed write therefore stops recording that session:

- A turn already streaming finishes; the agent's work is real whether or not the file kept it.
- A turn whose own prompt could not be recorded is refused before the agent is called.
- The Session moves to `historyState: degraded` with the OS reason, and later prompts fail with
  `session_history_degraded` until it is resumed.
- `resumeSessionHistory` appends a `Gap` record naming what interrupted the file before accepting new
  content, then makes the Session writable again. It records the loss; it does not restore it.
- A history file that cannot be read at all degrades the Session the same way, because appending
  without knowing which positions are used would overwrite them. A load whose history cannot be read
  fails instead of showing an empty conversation.

## Deletion

Soft deletion is what a user experiences as deletion, so the conversation goes with it: deleting a
Session removes its history file, and task and project cascades remove the files of every session
they take. Session ids are collected before the cascade because afterwards nothing links the files to
the task. Removal is best effort: the rows are already gone and an orphaned file is unreachable.

Session deletion serializes against new actor operations, unloads the route, soft-deletes the row and
then removes the file. ACP `session/delete` is used only to roll back a provider session that never
became visible; user-visible provider sessions are closed, never deleted, so provider-side history
survives. Reading a conversation does not make it `Running`, so only sessions actually attached hold a
task or project deletion off.
