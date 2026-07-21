# Brainstorm: ACP execution-flow mock

## Goal

Extend the ACP-backed chat prototype from streamed text replies into a complete,
demonstrable agent execution flow. The mock should exercise the same domain and
UI boundaries that a real ACP backend will use later, without touching the real
filesystem or speculating about the backend transport.

The agreed vertical slice is:

`thought -> plan -> read tool -> plan update -> edit tool with diff -> plan completion -> final response`

## Repository findings

- Commit `2857064` introduced the current ACP-backed conversation flow.
- The current `AcpClient` supports session creation, prompting, and update
  subscriptions. The mock only streams text `agent_message_chunk` updates.
- `packages/contracts/src/acp` already defines thought chunks, plans, tool calls,
  tool call updates, diffs, cancellation, stop reasons, and rich content blocks.
- `packages/chat` currently discards every session update except text agent
  message chunks and models a conversation as a flat message list.
- `packages/app-shell` only renders user and assistant text plus a typing
  indicator. Long thought-only periods therefore look stalled even though the
  prompt remains active.
- The production web runtime currently installs an unavailable ACP client. No
  real JSON-RPC/WebSocket/Tauri ACP adapter exists yet.
- The repository has collapsible UI primitives but no diff renderer or text
  diff dependency.

## Agreed scope

### Mock scenarios

The mock keeps the existing text-only response for ordinary questions. A small,
deterministic bilingual intent resolver selects richer scenarios for prompts
that express operations such as create, implement, modify, fix, or refactor.
Failure phrases take precedence over success phrases.

Scenario selection is injected through a resolver dependency. Runtime code uses
the default Chinese/English keyword resolver, while tests inject a resolver that
directly returns a scenario such as `chat`, `tool_success`, or `tool_failure`.
There are no hidden user-facing mock commands.

The successful scenario uses fixed virtual files and emits a stable event
sequence:

1. Stream a text thought in chunks.
2. Emit a complete plan snapshot with pending and in-progress entries.
3. Emit a read tool call and update it through its lifecycle.
4. Replace the plan snapshot with updated statuses.
5. Emit an edit tool call whose result contains `oldText` and `newText`.
6. Complete the plan.
7. Stream the final assistant response.

The failure scenario attempts to access a missing virtual file, marks the tool
as failed, leaves the relevant plan item incomplete, and streams an assistant
explanation. The prompt still ends normally because the failure belongs to the
tool, not the ACP transport.

### Virtual filesystem

Tool scenarios use a session-scoped in-memory filesystem. Reads observe prior
mock edits within the same ACP session, while sessions remain isolated. The
default scenario operates on fixed paths such as `src/app.ts`; test-related
prompts may include the fixed `src/app.test.ts` fixture.

The mock never reads or writes the user's real project. Virtual files and chat
execution state reset when the mock client is recreated.

### Timing and cancellation

Thought and final response text stream in small chunks. Plan and tool lifecycle
transitions use stable delays of roughly 150-250 ms so progress is visible
without making the demo slow. Every delay goes through the existing injectable
scheduler, and no random timing is introduced.

The composer exposes a stop command while a turn is active. `AcpClient` gains
the baseline ACP cancellation operation. The mock checks cancellation between
stages and chunks, stops emitting subsequent work, marks active tools as failed
with a cancelled explanation, preserves the current plan snapshot, and resolves
the prompt with `stopReason: "cancelled"`.

## Chat domain model

Replace the flat `ChatMessage[]` model with turn-scoped, ordered conversation
items. The initial item union contains:

- `message` for user and assistant content;
- `thought` for streamed agent progress;
- `plan` for the current full plan snapshot;
- `toolCall` for one tool lifecycle keyed by `toolCallId`.

Each item has a stable identity and belongs to a response turn. Repeated plan
notifications replace the current turn's plan in place. Tool updates replace
the matching tool in place while preserving first-seen order. Although the mock
runs tools sequentially, the state model must support multiple concurrent tool
IDs without overwriting them.

Each assistant turn has an explicit lifecycle:

- `streaming` while protocol updates are arriving;
- `completed` after a normal prompt response, retaining its `stopReason`;
- `cancelled` after user cancellation;
- `failed` after a rejected ACP request or transport error.

Conversation busy state is derived from the active turn instead of being stored
as an independent boolean. A failed tool may still belong to a completed turn
when the agent handles the failure and returns normally.

`ContentChunk.messageId` is optional in ACP. Explicit IDs are aggregated by ID.
When an ID is absent, the current turn maintains separate implicit thought and
assistant message items. A valid ID-less chunk must not become a conversation
error.

This slice fully renders text content and tool diffs. Other content blocks are
represented by a safe unsupported-content item instead of being silently
dropped or crashing the store. The mock itself emits only text and diff content.

## Frontend behavior

All activity for one response turn is visually grouped under one agent identity:

`thought -> plan -> tools -> final response`

- Thought is shown as subdued, collapsible progress. It is visible while
  streaming and automatically collapses when a tool call or final response
  begins.
- Plan is an inline, collapsible progress section at the start of the response.
  Updates replace the existing entries. It starts expanded and collapses to a
  completed-count summary when all entries finish.
- Tool calls are ordered by first appearance and update in place. Pending and
  running tools are expanded, completed tools collapse to a compact summary,
  and failed tools remain expanded.
- Tool summaries use the ACP tool kind, title, status, and file locations.
  Human-readable structured content is primary. Optional `rawInput` and
  `rawOutput` appear in a secondary formatted-JSON disclosure and are omitted
  when absent.
- Edit results use a compact unified diff suitable for the narrow conversation
  pane. The header shows the path and addition/deletion counts; changed hunks
  include line numbers and context, with an action to reveal the full diff.
  Use a maintained diff library rather than implementing a diff algorithm.
- File paths are interactive-looking locations, but opening the real editor is
  outside this slice.
- Unsupported rich content displays a clear placeholder.
- `end_turn` adds no notice. `cancelled` shows a subdued stopped notice.
  `max_tokens` and `max_turn_requests` show an incomplete-response warning, and
  `refusal` uses neutral treatment. Only rejected ACP operations use the turn
  failure UI.

## Backend replacement boundary

The frontend must depend only on the transport-independent `AcpClient`. Mock
scenario types, virtual files, and schedulers must not leak into `packages/chat`
or `packages/app-shell`.

A future backend adapter will own:

- transport connection and ACP initialization;
- JSON-RPC request IDs and request/response correlation;
- encoding session new, prompt, and cancel operations;
- forwarding `session/update` notifications to subscribers;
- transport errors, disconnect behavior, and capability negotiation;
- mapping stable Ora sessions to ACP agent sessions.

Application runtime composition remains the only replacement point. Connecting
the backend should replace the unavailable or mock ACP client with the real
adapter without changing the chat state machine or execution UI.

Do not add a speculative real transport in this change because the backend's
event channel has not been selected. Instead, define a shared `AcpClient`
conformance test harness. The mock and every future real adapter must pass the
same behavioral tests for event delivery, cancellation, errors, subscriptions,
and session isolation.

## Ownership

| Package | Responsibility |
|---|---|
| `packages/mock-service` | Scenario resolution, virtual files, deterministic event orchestration, and cancellation |
| `packages/chat` | Transport-independent ACP client contract, turn model, update normalization, and lifecycle state |
| `packages/app-shell` | Thought, plan, tool, diff, stop, and stop-reason presentation |
| `packages/ui` | Existing generic primitives only; no ACP-specific components |
| `packages/contracts` | Generated ACP types; do not hand-edit them for this feature |

## Test strategy

- Mock tests cover ordinary chat, successful and failed tool scenarios,
  cancellation, deterministic event order, session isolation, and stateful
  virtual file edits.
- Chat tests cover chunk aggregation, ID-less chunks, plan replacement, parallel
  tool IDs, tool failures, turn lifecycle, cancellation, stop reasons, and
  unsupported content.
- App-shell tests cover live, completed, failed, and cancelled rendering;
  thought and plan collapse rules; tool disclosures; unified diff output; and
  the stop interaction.
- Existing text-only conversation behavior remains covered as a regression.
- No browser E2E framework is added in this slice.

## Non-goals

- Permission request UI and reverse RPC.
- Embedded terminals.
- Session mode and configuration controls.
- Slash commands and usage/cost display.
- ACP session list, load, resume, or persisted conversation history.
- Real filesystem mutation or editor navigation.
- Full image, audio, resource, or terminal content rendering.
- A real backend transport adapter.

## Acceptance criteria

- A normal prompt still produces the existing streamed text conversation.
- An operation prompt visibly progresses through thought, plan, read, edit,
  diff, plan completion, and final response without duplicate timeline items.
- A failed tool and a cancelled turn remain understandable and leave the
  composer usable.
- Concurrent sessions and concurrent tool IDs do not corrupt each other's
  state.
- The mock has no real filesystem side effects.
- The mock passes the shared `AcpClient` conformance harness.
- Replacing the mock with a conforming real adapter requires no changes to the
  chat domain model or execution-flow components.

## Implementation status

Implemented on `frontend_0720` across `packages/mock-service`, `packages/chat`,
and `packages/app-shell`. The implementation includes the shared ACP client
conformance exercise, deterministic success/failure/cancellation scenarios,
turn-scoped chat state, execution-flow UI, and focused tests.

Verification completed with `cargo fmt --all`, the full `task test` workflow,
and production builds for both `@ora/web-client` and `@ora/desktop`.

## Follow-up: compact read activity

Long runs of tools should not render as a stack of equally prominent cards. The
frontend groups two or more adjacent calls by user intent: `read/search/fetch`
as exploration, `edit/move/delete` as file changes, and `execute` as command
batches. Each compact summary communicates aggregate status and relevant
context, while a disclosure preserves every original call and protocol detail.

Grouping is presentation-only: the chat store retains ordered ACP tool calls
unchanged. A different activity category, message, thought, or plan breaks a
group. Change summaries include unique file names and aggregate diff counts;
command summaries preview their human-readable titles. Active or failed groups
start expanded, completed groups collapse automatically, and individual
results remain independently expandable. `think`, `switch_mode`, and unknown
tools remain standalone because grouping them would obscure their meaning.

The successful mock scenario exercises every grouped presentation path without
adding UI-only switches: it emits two reads, two edits with structured diffs,
and two simulated command lifecycles. Commands only publish ACP events and
never invoke the host shell. Fixture paths, source text, titles, commands, and
outputs remain owned by `packages/mock-service`; the production presentation
continues to depend solely on ACP kinds, statuses, locations, and content.
