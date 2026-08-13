# ora-history

Ora's durable record of what a conversation was.

Ora is an ACP client, so the model context behind a session lives inside the agent CLI serving it. Owning the transcript separately is what lets a conversation outlive one provider: it can be replayed without asking the agent to recite it, and it can be handed to a different agent entirely.

## Responsibilities

- Assemble a live ACP `SessionUpdate` stream into settled history records, merging streamed chunks into whole messages.
- Append those records to one append-only JSONL file per Ora session.
- Read a file back into conversation order, tolerating a write that a crash cut short.
- Render a recorded conversation as the handoff block sent to a different agent CLI.

## Non-responsibilities

- No ACP work. The crate never speaks to a provider, and nothing here decides when a session loads, prompts, or switches.
- No database access. Session rows, their lifecycle, and their degraded-write state belong to `ora-backend` and `ora-db`.
- No policy. Which agent a conversation moves to, and when a transcript is injected, is the runtime's decision.

## Public boundary

`HistoryAssembler` turns updates into `AssembledRecord`s. `HistoryWriter` appends them and `remove_session_history` deletes a file. `read_session_history` returns a `SessionHistory`. `render_handoff` turns one into the block another agent receives, and `binding_needs_handoff` answers whether the session's current agent has already been given it. `history_path` derives where a session's file lives. Everything else is private.

## File layout

`<root>/<id[0..2]>/<id[2..4]>/<session-id>.jsonl`, sharded the way Git shards its object store. Session identifiers are random UUID v4 values, so the leading hex characters distribute evenly. The path is derived, never stored: a recorded location would be a second thing that can disagree about where a session's history lives.

## Record model

Every line is a `HistoryLine`: a local RFC 3339 timestamp, a position, and one `HistoryRecord`.

`Update` carries assembled ACP updates — one per settled message, thought, tool call, or plan — so replaying a file reproduces the conversation with no chunk merging on the read side. `Meta` opens the file. `TurnEnded` closes a prompt turn with its stop reason, which is the one thing provider replay never carried: without it a cancelled turn is indistinguishable from a completed one. `AgentSwitched` records a move to another CLI, and `HandoffDelivered` records that the agent it moved to was given the transcript. `Gap` marks content a failed write lost.

Agent identities are stored as the same namespaced text the database uses, not as Rust variant names, so an archived file does not depend on how the enum is spelled today.

## Invariants

- **Single writer.** Exactly one writer may exist per file, guaranteed structurally: the session's runtime actor owns it and serializes every operation on that session. Two Ora processes sharing one data root would violate this and is not supported.
- **Positions, not file order, define the timeline.** Records are appended when they settle rather than when they appeared, so a tool call that opened early and finished late is written after a message that started later. Readers sort on the position to repair this.
- **A repeated position means a correction.** A tool call that changes after being written is appended again under its original position. Readers keep the last record for a position.
- **Chunks are never stored.** A message reaches the file once, whole. Text merges by ACP `messageId`; a changed id settles the previous message. Text carrying no id has no identity to merge on, so contiguity stands in for one: the run is closed by any entry that takes its own position — a tool call, a plan, an image — and otherwise by the turn ending. Without that, an agent omitting `messageId` would collapse a whole turn's text into one record anchored at its first chunk, recorded ahead of every tool call it describes.
- **A tool call is settled only by evidence, never by assumption.** ACP does not require an agent to report a terminal status, and one that simply moves on leaves the call frozen at `pending` or `in_progress` — a state no reader can tell apart from work still in flight. A turn the agent ended on its own terms is evidence the call it left open ran to completion, and only there is it recorded as completed. Every other ending cut the turn short, so the unfinished status stands and `TurnEnded` carries the reason; the runtime records a broken connection and a failed prompt as cancelled, so those are covered by the same rule. A terminal status the agent did report is never reinterpreted. The inference is deliberate and lossy: a completed status in the file may be the agent's report or Ora's, and the two are not distinguishable after the fact.

## Failure semantics

- A missing file is an empty history, not an error. A session that was never prompted has nothing recorded, and neither does one created before Ora owned its own history.
- A final line left unterminated by an interrupted write is discarded silently — that is the expected shape of a crash. Any other unparseable line is dropped but counted in `dropped_lines`, because it lost content that no longer has a place in the timeline and the caller must be able to say so.
- Writes are flushed, not synced. Losing the last records to a power cut is an accepted trade for keeping a long turn's appends off the disk's latency path.
- Deleting a session's history removes its file and leaves its two shard directories in place. They are shared with every other session whose identifier starts the same way, so removing one that looks empty would race a session being created alongside it.
- Every write error is a reason to stop writing that session, never to retry silently. A history that skips records is more dangerous than one that stops, because the gap is invisible to whoever replays it — see the degraded-write handling in [ACP Agent Runtime](../../docs/agent-runtime.md).

## Handoff rendering

ACP cannot install a conversation into an agent: `session/new` takes no context and `session/prompt` takes a single user turn. Every recorded turn therefore collapses into one user message, and the receiving agent sees a transcript it is being shown rather than one it took part in. The preamble exists to say so.

The transcript keeps user messages and assistant replies in full and reduces each tool call to its title and outcome. Reasoning, tool inputs and outputs, plans, and session chrome are dropped: they belong to the agent that produced them, are stale on arrival, or would crowd out the conversation itself. Text that contains the block's own markers is neutralized so a transcript cannot close the section wrapping it.

There is no size budget. A long conversation can exceed the receiving model's context window, and that failure surfaces from the provider.

`binding_needs_handoff` answers whether the agent currently bound to a session has been shown what came before it, reading the most recent of the two records that can settle the question: `HandoffDelivered` means yes, `AgentSwitched` with nothing after it means no. A user turn deliberately settles nothing. The runtime records a prompt before sending it, so a user turn sitting after a switch is equally consistent with a delivered transcript and with a `session/prompt` that never left the process — reading it as proof of delivery is what would lose a transcript permanently. A switch whose delivery the file cannot prove is therefore read as still owed, because handing a transcript over twice costs a repeated history while skipping it leaves an agent answering a conversation it cannot see. A file with no switch at all owes nothing: there is no binding to bring up to date.

## Interactions

`ora-backend`'s agent runtime owns every instance of these types, supplies the clock, and decides when to write, replay, hand off, or delete. The official `agent-client-protocol-schema` crate supplies the ACP vocabulary the records are built from, `ora-contracts` supplies Ora's application events, and `ora-domain` supplies the CLI identity. See [ACP Agent Runtime](../../docs/agent-runtime.md).
