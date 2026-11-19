# Subagent Details

English | [中文](subagent-details.zh.md)

The parent conversation can show read-only child-session cards for OpenCode inline projections and
CodeAgent `_subagent/update` notifications. Expand a card to inspect
its text, thoughts, plans and tool input/output; OpenCode nested cards follow the explicit parent
relationship, while `_subagent/update` notifications carry no nested parent chain and flatten as
`depth: 1` sibling cards. Parent replies remain in the ordinary timeline. The original Stop action controls the parent prompt;
these cards do not add child cancellation, steering or closing.

Child state is unreported while its parent is still observed, unless a supported report establishes it.
After a parent completion, failure or cancellation, the card shows **Delegation returned** only when
either (a) the latest OpenCode delegation call in the same parent turn and source stream names this
child in `rawOutput.metadata.sessionID`, reports ACP `completed`, and carries the provider's `status`
of `completed`, or (b) `_subagent/update` explicitly linked the group to a parent `toolCallId` whose
latest call in the parent stream completed with provider output (`content` or `rawOutput`) rather than
turn-end settlement, and without a non-`completed` `rawOutput.status` or `rawOutput.metadata.status`. Every other case shows **Further state unknown (projection stopped)**: an unfinished,
failed, cancelled or background `status=running` delegation, or no associable session id. The latest
call is chosen by persisted position, so a later turn continuing the same child session never revises
an earlier turn. Both labels describe a returned foreground result or missing subsequent observations,
never a completed, cancelled or stopped child; live, warm and cold replay derive the same label from the
same persisted facts. Historical EOF without a recorded turn fence remains unknown instead of inventing
that ending. An unfinished child tool displays an unknown outcome when the parent turn ends. Missing or
cyclic relationships remain independent readable cards and never claim a delegation result.

Attribution comes from `_meta["opencode/child-session"]` or `_meta["_subagent/update"]` (normalized from
the ACP `_subagent/update` extension notification), never from a tool title, ID prefix, or depth
alone. Unsupported or malformed sources remain in the parent timeline. Complete metadata and text
annotations are retained during live batching and replay; distinct sources cannot merge on colliding
or missing message IDs. Incremental tool metadata replaces supplied top-level namespaces, including
explicit null, without recursively patching unknown provider objects.

While a linked delegation is still `pending` or `in_progress` in the current attempt, its
`_subagent/update` frames renew the parent prompt's inactivity window without pausing it, keeping a
foreground delegation alive as long as frames arrive within each window (a single silent subagent
tool exceeding the window still triggers cancel/retry, and continuous frames renew until stopped);
see [Agent runtime](agent-runtime.md#prompt-inactivity-and-retries).

The [history owner](session-history.md) must persist the source metadata. This UI cannot recover
attribution omitted from older files, and ordinary OpenCode projection may stop when the parent prompt
ends. Cards therefore state that only received content is available and do not promise complete
background history. Live observation diagnostics and historical content are separate concerns.
