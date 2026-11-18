import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import { createChatStore, type ChatTurn } from "@ora/chat";
import type { LoadSessionEvent, PromptSessionEvent } from "@ora/contracts";
import { createTestClient } from "../../test/contracts-transport";
import { appI18n } from "../../i18n/i18n-instance";
import { ResponseTurn } from "./response-turn";

beforeEach(async () => {
  await act(() => appI18n.changeLanguage("en-US"));
});
const meta = (id: string, parentID: string, depth: number, title: string) => ({
  "opencode/child-session": { id, parentID, depth, title },
});
const worker = meta("worker", "root", 1, "Outer worker");
const nested = meta("nested", "worker", 2, "Nested worker");
const live: PromptSessionEvent[] = [
  {
    type: "session_update",
    update: {
      sessionUpdate: "agent_thought_chunk",
      messageId: "thought",
      content: { type: "text", text: "I will delegate" },
      _meta: worker,
    },
  },
  {
    type: "session_update",
    update: {
      sessionUpdate: "tool_call",
      toolCallId: "worker:pending",
      title: "Pending probe",
      kind: "execute",
      status: "in_progress",
      rawInput: { command: "sleep 60" },
      _meta: worker,
    },
  },
  {
    type: "session_update",
    update: {
      sessionUpdate: "tool_call",
      toolCallId: "nested:shell",
      title: "Probe command",
      kind: "execute",
      status: "completed",
      rawInput: { command: "echo proof" },
      rawOutput: { exit: 0 },
      content: [{ type: "content", content: { type: "text", text: "proof" } }],
      _meta: nested,
    },
  },
  {
    type: "session_update",
    update: {
      sessionUpdate: "agent_message_chunk",
      messageId: "nested-result",
      content: { type: "text", text: "NESTED_RESULT" },
      _meta: nested,
    },
  },
  {
    type: "session_update",
    update: {
      sessionUpdate: "agent_message_chunk",
      messageId: "worker-result",
      content: { type: "text", text: "WORKER_RESULT" },
      _meta: worker,
    },
  },
  {
    type: "session_update",
    update: {
      sessionUpdate: "agent_message_chunk",
      messageId: "parent-result",
      content: { type: "text", text: "PARENT_RESULT" },
    },
  },
  { type: "completed", stopReason: "end_turn" },
];
async function* stream<T>(events: T[]) {
  for (const event of events) yield structuredClone(event);
}

it("renders nested read-only details from the production store and keeps cold replay attribution", async () => {
  const cold: LoadSessionEvent[] = [
    {
      type: "session_update",
      update: {
        sessionUpdate: "user_message_chunk",
        content: { type: "text", text: "go" },
      },
    },
    ...live.flatMap((event): LoadSessionEvent[] =>
      event.type === "session_update" ? [event] : [],
    ),
    { type: "turn_ended", stopReason: "end_turn" },
    { type: "completed" },
  ];
  const client = createTestClient({
    promptSession: () => stream(live),
    loadSession: () => stream(cold),
  });
  const store = createChatStore(client.session, { now: () => 100 });
  await store.getState().sendMessage({ oraSessionId: "live", text: "go" });
  await store.getState().loadSession("cold");
  const summary = (id: string) =>
    store.getState().conversations[id]!.turns[0]!.items.map((item) => ({
      kind: item.kind,
      metadata: item.metadata,
      content: "content" in item ? item.content : undefined,
    }));
  expect(summary("cold")).toEqual(summary("live"));
  const user = userEvent.setup();
  render(
    <ResponseTurn
      turn={store.getState().conversations.cold!.turns[0]!}
      userName="Reader"
    />,
  );
  const outer = screen.getByRole("region", { name: "Subagent · Outer worker" });
  await user.click(
    within(outer).getByRole("button", { name: /Subagent · Outer worker/ }),
  );
  const child = screen.getByRole("region", {
    name: "Subagent · Nested worker",
  });
  await user.click(
    within(child).getByRole("button", { name: /Subagent · Nested worker/ }),
  );
  expect(within(child).getByText("NESTED_RESULT")).toBeInTheDocument();
  expect(within(outer).getByText("WORKER_RESULT")).toBeInTheDocument();
  expect(within(outer).queryByText("PARENT_RESULT")).not.toBeInTheDocument();
  expect(
    screen.getAllByText("Further state unknown (projection stopped)"),
  ).toHaveLength(2);
  expect(
    screen.queryByRole("button", { name: /steer|cancel child|close child/i }),
  ).not.toBeInTheDocument();
  store.getState().dispose();
});

/** Opens only the outer card; nested status remains visible without opening nested contents. */
async function expectStoppedProjection(turn: ChatTurn) {
  const view = render(<ResponseTurn turn={turn} userName="Reader" />);
  const outer = screen.getByRole("region", { name: "Subagent · Outer worker" });
  await userEvent.click(
    within(outer).getByRole("button", { name: /Subagent · Outer worker/ }),
  );
  expect(
    Array.from(outer.querySelectorAll("[data-subagent-status]")).map(
      (node) => node.textContent,
    ),
  ).toEqual([
    "Further state unknown (projection stopped)",
    "Further state unknown (projection stopped)",
  ]);
  // The unfinished child tool is settled by the observed parent ending, and its never-reported
  // outcome must stay unknown instead of borrowing the parent's completion.
  const pendingTool = outer.querySelector(
    '[data-child-session="worker"][data-subagent-item="toolCall"]',
  );
  expect(pendingTool).not.toBeNull();
  expect(pendingTool).toHaveTextContent("Outcome unknown");
  expect(pendingTool).not.toHaveTextContent("Completed");
  view.unmount();
}

it.each(["completed", "failed", "cancelled"] as const)(
  "keeps missing child terminal state explicit after parent %s in live, warm and cold views",
  async (ending) => {
    const updates = live.filter(
      (
        event,
      ): event is Extract<PromptSessionEvent, { type: "session_update" }> =>
        event.type === "session_update",
    );
    const stopReason = ending === "completed" ? "end_turn" : "cancelled";
    const history: LoadSessionEvent[] = [
      {
        type: "session_update",
        update: {
          sessionUpdate: "user_message_chunk",
          content: { type: "text", text: "go" },
        },
      },
      ...updates,
      { type: "turn_ended", stopReason },
      { type: "completed" },
    ];
    const client = createTestClient({
      async *promptSession() {
        yield* stream(updates);
        if (ending === "failed") throw new Error("parent stream failed");
        yield { type: "completed", stopReason } satisfies PromptSessionEvent;
      },
      loadSession: () => stream(history),
    });
    const store = createChatStore(client.session, { now: () => 100 });
    const request = store
      .getState()
      .sendMessage({ oraSessionId: "one", text: "go" });
    if (ending === "failed") await expect(request).rejects.toThrow();
    else await request;
    await expectStoppedProjection(
      store.getState().conversations.one!.turns[0]!,
    );
    await store.getState().loadSession("one");
    await expectStoppedProjection(
      store.getState().conversations.one!.turns[0]!,
    );
    const cold = createChatStore(client.session, { now: () => 999 });
    await cold.getState().loadSession("one");
    await expectStoppedProjection(cold.getState().conversations.one!.turns[0]!);
    store.getState().dispose();
    cold.getState().dispose();
  },
);

it("keeps a settled child projection unknown after the same session starts a new prompt", async () => {
  let calls = 0;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const client = createTestClient({
    async *promptSession() {
      calls += 1;
      if (calls === 1) {
        yield* stream(live);
        return;
      }
      // The second prompt stays open long enough to prove the first turn keeps its own fence.
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "agent_message_chunk",
          messageId: "later",
          content: { type: "text", text: "LATER" },
          _meta: worker,
        },
      } satisfies PromptSessionEvent;
      await gate;
      yield {
        type: "completed",
        stopReason: "end_turn",
      } satisfies PromptSessionEvent;
    },
  });
  const store = createChatStore(client.session, { now: () => 100 });
  await store.getState().sendMessage({ oraSessionId: "one", text: "first" });
  const second = store.getState().sendMessage({
    oraSessionId: "one",
    text: "second",
  });
  await waitFor(() => {
    const turns = store.getState().conversations.one!.turns;
    if (turns.length !== 2 || turns[1]!.items.length === 0) {
      throw new Error("waiting for the second prompt's first child update");
    }
  });
  expect(store.getState().conversations.one!.turns[1]!.status).toBe(
    "streaming",
  );
  const first = render(
    <ResponseTurn
      turn={store.getState().conversations.one!.turns[0]!}
      userName="Reader"
    />,
  );
  expect(screen.queryByText("State not reported (observing)")).toBeNull();
  expect(
    screen.getAllByText("Further state unknown (projection stopped)"),
  ).toHaveLength(1);
  first.unmount();
  const later = render(
    <ResponseTurn
      turn={store.getState().conversations.one!.turns[1]!}
      userName="Reader"
    />,
  );
  expect(screen.getAllByText("State not reported (observing)")).toHaveLength(1);
  later.unmount();
  release();
  await second;
  store.getState().dispose();
});

it.each([true, false])(
  "does not invent a stopped projection from old EOF-only history (attribution=%s)",
  async (attributed) => {
    const history: LoadSessionEvent[] = [
      {
        type: "session_update",
        update: {
          sessionUpdate: "agent_message_chunk",
          content: { type: "text", text: "Old content" },
          ...(attributed ? { _meta: worker } : {}),
        },
      },
      { type: "completed" },
    ];
    const client = createTestClient({ loadSession: () => stream(history) });
    const store = createChatStore(client.session);
    await store.getState().loadSession("old");
    const turn = store.getState().conversations.old!.turns[0]!;
    expect(turn.stopReason).toBeNull();
    const view = render(<ResponseTurn turn={turn} userName="Reader" />);
    expect(
      screen.queryByText("Further state unknown (projection stopped)"),
    ).not.toBeInTheDocument();
    if (attributed)
      expect(screen.getByText("Status unknown")).toBeInTheDocument();
    else
      expect(
        screen.queryByRole("region", { name: /Subagent/ }),
      ).not.toBeInTheDocument();
    view.unmount();
    store.getState().dispose();
  },
);

/** Builds the parent-side events that delegate to one child and report the provider's result. */
function delegation(
  toolCallId: string,
  sessionID: string,
  status: string,
  meta?: Record<string, unknown>,
): PromptSessionEvent[] {
  return [
    {
      type: "session_update",
      update: {
        sessionUpdate: "tool_call",
        toolCallId,
        title: "subagent",
        kind: "think",
        status: "in_progress",
        rawInput: { prompt: "work" },
        ...(meta ? { _meta: meta } : {}),
      },
    },
    {
      type: "session_update",
      update: {
        sessionUpdate: "tool_call_update",
        toolCallId,
        status: "completed",
        rawOutput: { metadata: { sessionID, status } },
      },
    },
  ];
}

const childText = (
  id: string,
  body: string,
  meta: Record<string, unknown>,
): PromptSessionEvent => ({
  type: "session_update",
  update: {
    sessionUpdate: "agent_message_chunk",
    messageId: id,
    content: { type: "text", text: body },
    _meta: meta,
  },
});

/** Renders one ended turn and returns the status labels of its visible subagent cards. */
async function childStatusLabels(
  turn: ChatTurn,
  openOuter = false,
): Promise<string[]> {
  const view = render(<ResponseTurn turn={turn} userName="Reader" />);
  if (openOuter) {
    // Nested cards mount only after their parent card is expanded.
    await userEvent.click(
      screen.getByRole("button", { name: /Subagent · Outer worker/ }),
    );
  }
  const labels = Array.from(
    document.querySelectorAll("[data-subagent-status]"),
  ).map((node) => node.textContent);
  view.unmount();
  return labels;
}

it("settles a child only from the delegation that names it in live, warm and cold views", async () => {
  const updates = [
    ...delegation("outer-delegate", "worker", "completed"),
    childText("worker-result", "WORKER_RESULT", worker),
    ...delegation("inner-delegate", "nested", "completed", worker),
    childText("nested-result", "NESTED_RESULT", nested),
  ].flatMap(
    (event): Extract<PromptSessionEvent, { type: "session_update" }>[] =>
      event.type === "session_update" ? [event] : [],
  );
  const history: LoadSessionEvent[] = [
    {
      type: "session_update",
      update: {
        sessionUpdate: "user_message_chunk",
        content: { type: "text", text: "go" },
      },
    },
    ...updates,
    { type: "turn_ended", stopReason: "end_turn" },
    { type: "completed" },
  ];
  const client = createTestClient({
    async *promptSession() {
      yield* stream(updates);
      yield {
        type: "completed",
        stopReason: "end_turn",
      } satisfies PromptSessionEvent;
    },
    loadSession: () => stream(history),
  });
  const store = createChatStore(client.session, { now: () => 100 });
  await store.getState().sendMessage({ oraSessionId: "one", text: "go" });
  // The foreground delegation returned, and the nested one is read from its own parent stream;
  // neither call settles the child, so only the "returned" wording may appear.
  expect(
    await childStatusLabels(
      store.getState().conversations.one!.turns[0]!,
      true,
    ),
  ).toEqual(["Delegation returned", "Delegation returned"]);
  await store.getState().loadSession("one");
  expect(
    await childStatusLabels(
      store.getState().conversations.one!.turns[0]!,
      true,
    ),
  ).toEqual(["Delegation returned", "Delegation returned"]);
  const cold = createChatStore(client.session, { now: () => 999 });
  await cold.getState().loadSession("one");
  expect(
    await childStatusLabels(cold.getState().conversations.one!.turns[0]!, true),
  ).toEqual(["Delegation returned", "Delegation returned"]);
  store.getState().dispose();
  cold.getState().dispose();
});

it("keeps a background delegation's child on the projection-stopped wording", async () => {
  const updates = [
    ...delegation("outer-delegate", "worker", "running"),
    childText("worker-result", "WORKER_RESULT", worker),
  ].flatMap(
    (event): Extract<PromptSessionEvent, { type: "session_update" }>[] =>
      event.type === "session_update" ? [event] : [],
  );
  const client = createTestClient({
    async *promptSession() {
      yield* stream(updates);
      yield {
        type: "completed",
        stopReason: "end_turn",
      } satisfies PromptSessionEvent;
    },
    loadSession: () =>
      stream<LoadSessionEvent>([
        {
          type: "session_update",
          update: {
            sessionUpdate: "user_message_chunk",
            content: { type: "text", text: "go" },
          },
        },
        ...updates,
        { type: "turn_ended", stopReason: "end_turn" },
        { type: "completed" },
      ]),
  });
  const store = createChatStore(client.session, { now: () => 100 });
  await store.getState().sendMessage({ oraSessionId: "one", text: "go" });
  expect(
    await childStatusLabels(store.getState().conversations.one!.turns[0]!),
  ).toEqual(["Further state unknown (projection stopped)"]);
  await store.getState().loadSession("one");
  expect(
    await childStatusLabels(store.getState().conversations.one!.turns[0]!),
  ).toEqual(["Further state unknown (projection stopped)"]);
  const cold = createChatStore(client.session, { now: () => 999 });
  await cold.getState().loadSession("one");
  expect(
    await childStatusLabels(cold.getState().conversations.one!.turns[0]!),
  ).toEqual(["Further state unknown (projection stopped)"]);
  store.getState().dispose();
  cold.getState().dispose();
});

it("keeps an earlier turn's returned delegation after the same child is resumed", async () => {
  let calls = 0;
  const first = [
    ...delegation("first-delegate", "worker", "completed"),
    childText("worker-result", "WORKER_RESULT", worker),
  ].flatMap(
    (event): Extract<PromptSessionEvent, { type: "session_update" }>[] =>
      event.type === "session_update" ? [event] : [],
  );
  const second = [
    ...delegation("second-delegate", "worker", "running"),
    childText("worker-later", "WORKER_LATER", worker),
  ].flatMap(
    (event): Extract<PromptSessionEvent, { type: "session_update" }>[] =>
      event.type === "session_update" ? [event] : [],
  );
  const client = createTestClient({
    async *promptSession() {
      calls += 1;
      yield* stream(calls === 1 ? first : second);
      yield {
        type: "completed",
        stopReason: "end_turn",
      } satisfies PromptSessionEvent;
    },
    loadSession: () =>
      stream<LoadSessionEvent>([
        {
          type: "session_update",
          update: {
            sessionUpdate: "user_message_chunk",
            content: { type: "text", text: "first" },
          },
        },
        ...first,
        { type: "turn_ended", stopReason: "end_turn" },
        {
          type: "session_update",
          update: {
            sessionUpdate: "user_message_chunk",
            content: { type: "text", text: "second" },
          },
        },
        ...second,
        { type: "turn_ended", stopReason: "end_turn" },
        { type: "completed" },
      ]),
  });
  const store = createChatStore(client.session, { now: () => 100 });
  await store.getState().sendMessage({ oraSessionId: "one", text: "first" });
  await store.getState().sendMessage({ oraSessionId: "one", text: "second" });
  const turns = store.getState().conversations.one!.turns;
  expect(await childStatusLabels(turns[0]!)).toEqual(["Delegation returned"]);
  expect(await childStatusLabels(turns[1]!)).toEqual([
    "Further state unknown (projection stopped)",
  ]);
  await store.getState().loadSession("one");
  const warm = store.getState().conversations.one!.turns;
  expect(await childStatusLabels(warm[0]!)).toEqual(["Delegation returned"]);
  expect(await childStatusLabels(warm[1]!)).toEqual([
    "Further state unknown (projection stopped)",
  ]);
  const cold = createChatStore(client.session, { now: () => 999 });
  await cold.getState().loadSession("one");
  const reloaded = cold.getState().conversations.one!.turns;
  expect(await childStatusLabels(reloaded[0]!)).toEqual([
    "Delegation returned",
  ]);
  expect(await childStatusLabels(reloaded[1]!)).toEqual([
    "Further state unknown (projection stopped)",
  ]);
  store.getState().dispose();
  cold.getState().dispose();
});

it("renders _subagent/update cards with agentType title and outer toolCallId return status across live and cold replay", async () => {
  const subagentMeta = {
    "_subagent/update": {
      sessionId: "main-session",
      taskId: "task-explore",
      toolCallId: "parent-agent-call",
      agentType: "Explore",
    },
  };
  const updates: Extract<PromptSessionEvent, { type: "session_update" }>[] = [
    {
      type: "session_update",
      update: {
        sessionUpdate: "tool_call",
        toolCallId: "parent-agent-call",
        title: "Agent",
        kind: "think",
        status: "pending",
        rawInput: { prompt: "Inspect codebase" },
      },
    },
    {
      type: "session_update",
      update: {
        sessionUpdate: "agent_thought_chunk",
        content: { type: "text", text: "Scanning directories" },
        _meta: subagentMeta,
      },
    },
    {
      type: "session_update",
      update: {
        sessionUpdate: "tool_call",
        toolCallId: "task-explore:read-1",
        title: "Read README",
        kind: "read",
        status: "completed",
        rawOutput: "# Ora",
        _meta: { ...subagentMeta, codeAgentCLI: { toolName: "Read" } },
      },
    },
    {
      type: "session_update",
      update: {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "CODEAGENT_SUBAGENT_SUMMARY" },
        _meta: subagentMeta,
      },
    },
    {
      type: "session_update",
      update: {
        sessionUpdate: "tool_call_update",
        toolCallId: "parent-agent-call",
        status: "completed",
        rawOutput: "Subagent report delivered",
      },
    },
  ];
  const history: LoadSessionEvent[] = [
    {
      type: "session_update",
      update: {
        sessionUpdate: "user_message_chunk",
        content: { type: "text", text: "explore" },
      },
    },
    ...updates,
    { type: "turn_ended", stopReason: "end_turn" },
    { type: "completed" },
  ];
  const client = createTestClient({
    async *promptSession() {
      yield* stream(updates);
      yield { type: "completed", stopReason: "end_turn" };
    },
    loadSession: () => stream(history),
  });
  const store = createChatStore(client.session, { now: () => 100 });
  await store.getState().sendMessage({ oraSessionId: "live", text: "explore" });
  const cold = createChatStore(client.session, { now: () => 999 });
  await cold.getState().loadSession("cold");

  for (const turn of [
    store.getState().conversations.live!.turns[0]!,
    cold.getState().conversations.cold!.turns[0]!,
  ]) {
    const user = userEvent.setup();
    const view = render(<ResponseTurn turn={turn} userName="Reader" />);
    const card = screen.getByRole("region", { name: "Subagent · Explore" });
    expect(within(card).getByText("Delegation returned")).toBeInTheDocument();
    await user.click(
      within(card).getByRole("button", { name: /Subagent · Explore/ }),
    );
    expect(
      within(card).getByText("CODEAGENT_SUBAGENT_SUMMARY"),
    ).toBeInTheDocument();
    expect(within(card).getByText("Read README")).toBeInTheDocument();
    view.unmount();
  }
  store.getState().dispose();
  cold.getState().dispose();
});
