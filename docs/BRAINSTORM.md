# Workflow node session conversation

## Goal

Let users inspect the conversation bound to a workflow node without leaving the
run workspace. The first implementation covers the frontend and memory mock
only, while keeping the read model replaceable by a backend session adapter.

## Codebase findings

- Theater already treats the focused node as a large stage card and uses card
  clicks to open a resizable right-side inspector.
- The inspector owns configuration, execution I/O, and artifacts; it should
  remain the detailed/debugging surface.
- `MessageBubble` and `MarkdownMessage` already provide the production chat
  message layout and Markdown renderer, so the node view should reuse them.
- HITL can occupy the card footer, and parallel nodes also have compact cards,
  so embedded conversation controls must not share those click targets or make
  the parallel carousel jump in height.
- The memory runtime already exposes an atomic live snapshot plus cursor-based
  events, which is the natural boundary for a future backend adapter.

## Interaction alternatives

### Expand downward

- Preserves the stage summary and makes the relationship easy to understand.
- Causes card and stage height changes, competes with embedded HITL, and can
  move the centered Theater composition or force extra scrolling.
- Best as a fallback for narrow layouts, not as the primary desktop behavior.

### Cover the existing card body

- Keeps the stage geometry stable and gives replies more reading room.
- A plain overlay can feel temporary or modal and may obscure which node the
  session belongs to.

### Morph the card into a conversation surface

- Recommended. Keep the outer card shell, node header, status, and lower-right
  session control stable; crossfade/slide only the card body between `summary`
  and `conversation` modes.
- This preserves node identity while making the session feel intrinsic to the
  node. It also avoids Theater reflow and gives Markdown enough width.
- Avoid a literal 3D flip. Use a short opacity + small vertical translation,
  and disable non-essential motion under `prefers-reduced-motion`.

## Current recommendation

Use a **session dock with a card-body morph** on the focused stage card.

- Add an attached, icon-only session control at the card's lower-right edge.
  A contained spark and message-count badge make it discoverable without a
  potentially confusing action label; it changes to a back affordance when
  open.
- Clicking the dock stops card propagation and switches the card body from its
  stage summary to a conversation reading surface. The shell and header remain
  in place. Clicking the rest of the summary card continues to open the
  existing stage inspector.
- Render user messages and backend-approved Agent messages with the production
  `MessageBubble`/`MarkdownMessage` pair. Keep thought and tool-call records in
  the projection as secondary activity, but fold consecutive activity into one
  disclosure so the default view stays focused on the conversation.
- Keep conversation mode visually quiet and chat-like: the bounded body is
  wheel-scrollable, long Markdown follows the full chat rhythm, and the
  embedded read-only view has no feedback actions or chat composer. Preserve a
  clear one-click return to the stage summary.
- Only the primary stage card expands. On a compact parallel card, the same
  control first promotes that node to the primary stage and opens its session
  dock, avoiding carousel height shifts.
- Empty/running states remain useful: show a short waiting state when the node
  session exists but has not emitted a formal message yet.

## Integration boundary

Expose a node-session conversation projection through the workflow runtime live
snapshot and upsert event stream. Visible message items carry run, node, and
session IDs, role, Markdown content, completion state, and timestamps; activity
items carry a compact kind/summary/detail for disclosure. The memory mock
produces both shapes, and a later backend adapter can derive the same
projection from the real session while retaining filtering at this boundary.

## Confirmed decision

Implement the card-body morph. The conversation contains node input, user HITL
submissions, formal Agent messages, and collapsed secondary activity. Remove
the duplicate input/output blocks from the right inspector, which remains
responsible for configuration, execution metrics, errors, and artifacts.

## Anchor navigation follow-up

- Reuse the formal chat navigator's scroll-following, active-anchor, wheel, and
  tail navigation behavior through the shared `useConversationNavigation` hook.
  `ConversationNavigator` now accepts generic anchors and a container placement,
  so full chat keeps its fixed minimap skin while the node card uses a compact
  in-card rail against the same scroll container. The node rail appears only
  after four visible message anchors; shorter node sessions stay visually quiet.
- The shared anchor model is (`id`, `role`, `label`, `preview`, `summary`),
  keeping navigation independent from `ChatTurn[]`.
- Build node anchors directly from stable conversation item IDs. Visible user
  and Agent messages become anchors; collapsed thought/tool activity stays out
  of the default anchor track until explicitly expanded.
- The backend only needs to preserve stable item IDs, role/kind, timestamps,
  session ID, and cursor-ordered updates. Streaming updates replace an item by
  ID, so anchor positions remain stable across reconnects.
# Implementation note: node conversation anchor highlights are rendered by the
# shared message bubble renderer. User bubbles and Agent Markdown each expose
# a matching highlight surface, and the animated SVG is layered above message
# backgrounds when a navigator jump focuses a message.
