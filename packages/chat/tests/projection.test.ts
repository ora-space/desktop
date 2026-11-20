import assert from "node:assert/strict";
import test from "node:test";
import type * as acp from "@agentclientprotocol/sdk";
import type { LoadSessionEvent, PromptSessionEvent } from "@ora/contracts";
import {
  childSessionSource,
  createChatStore,
  type ChatSessionClient,
  type ChatTurnItem,
} from "../src/index.ts";

const source = (id: string) => ({
  "opencode/child-session": { id, parentID: "root", depth: 1 },
});
const text = (
  id: string,
  body: string,
  messageId?: string,
): acp.SessionUpdate => ({
  sessionUpdate: "agent_message_chunk",
  ...(messageId ? { messageId } : {}),
  content: {
    type: "text",
    text: body,
    _meta: { format: "plain" },
    annotations: { audience: ["assistant"] },
  },
  _meta: source(id),
});

async function* events<T>(items: T[]) {
  yield* items;
}

test("live batching and reload keep colliding child text ids and annotations distinct", async () => {
  const updates = [
    text("left", "A", "shared"),
    text("right", "B", "shared"),
    text("left", "C", "shared"),
  ];
  const client: ChatSessionClient = {
    prompt: () =>
      events<PromptSessionEvent>([
        ...updates.map((update) => ({
          type: "session_update" as const,
          update,
        })),
        { type: "completed", stopReason: "end_turn" },
      ]),
    load: () =>
      events<LoadSessionEvent>([
        {
          type: "session_update",
          update: {
            sessionUpdate: "user_message_chunk",
            content: { type: "text", text: "go" },
          },
        },
        ...[text("left", "AC", "shared"), text("right", "B", "shared")].map(
          (update) => ({ type: "session_update" as const, update }),
        ),
        { type: "turn_ended", stopReason: "end_turn" },
        { type: "completed" },
      ]),
    setConfig: async () => ({ configOptions: [] }),
    respondToPermission: async () => ({}),
  };
  const store = createChatStore(client, { now: () => 42 });
  await store.getState().sendMessage({ oraSessionId: "live", text: "go" });
  await store.getState().loadSession("cold");
  const summary = (id: string) =>
    store.getState().conversations[id]!.turns[0]!.items.map((item) => ({
      kind: item.kind,
      metadata: item.metadata,
      context: item.contentContext,
      text: item.kind === "message" ? item.content : "",
    }));
  assert.deepEqual(summary("live"), summary("cold"));
  assert.deepEqual(
    summary("live").map((x) => x.text),
    ["AC", "B"],
  );
  store.getState().dispose();
});

test("late attribution, opaque replacement and unknown child outcomes survive finalization", async () => {
  const client: ChatSessionClient = {
    async *prompt() {
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "tool_call",
          toolCallId: "t",
          title: "Work",
          status: "in_progress",
        },
      } satisfies PromptSessionEvent;
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "tool_call_update",
          toolCallId: "t",
          _meta: { ...source("child"), other: { stale: true } },
        },
      } satisfies PromptSessionEvent;
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "tool_call_update",
          toolCallId: "t",
          _meta: { other: { fresh: true } },
        },
      } satisfies PromptSessionEvent;
      yield {
        type: "completed",
        stopReason: "end_turn",
      } satisfies PromptSessionEvent;
    },
    load: () => events<LoadSessionEvent>([]),
    setConfig: async () => ({ configOptions: [] }),
    respondToPermission: async () => ({}),
  };
  const store = createChatStore(client);
  await store.getState().sendMessage({ oraSessionId: "one", text: "go" });
  const item = store.getState().conversations.one!.turns[0]!.items[0]!;
  assert.equal(item.kind, "toolCall");
  assert.deepEqual(item.metadata, {
    ...source("child"),
    other: { fresh: true },
  });
  if (item.kind === "toolCall") assert.equal(item.status, "unknown");
  assert.equal(
    childSessionSource({ "opencode/child-session": null }),
    undefined,
  );
  store.getState().dispose();
});

test("cleared or invalid child sources settle as parent tools in live and reloaded turns", async () => {
  for (const invalid of [
    null,
    { id: " ", parentID: "root", depth: 1 },
    { id: "child", parentID: " ", depth: 1 },
    { id: "child", parentID: "root", depth: 1.5 },
  ]) {
    const meta = { "opencode/child-session": invalid };
    const client: ChatSessionClient = {
      prompt: () =>
        events<PromptSessionEvent>([
          {
            type: "session_update",
            update: {
              sessionUpdate: "tool_call",
              toolCallId: "tool",
              title: "Work",
              status: "in_progress",
              _meta: source("child"),
            },
          },
          {
            type: "session_update",
            update: {
              sessionUpdate: "tool_call_update",
              toolCallId: "tool",
              _meta: meta,
            },
          },
          { type: "completed", stopReason: "end_turn" },
        ]),
      load: () =>
        events<LoadSessionEvent>([
          {
            type: "session_update",
            update: {
              sessionUpdate: "user_message_chunk",
              content: { type: "text", text: "go" },
            },
          },
          {
            type: "session_update",
            update: {
              sessionUpdate: "tool_call",
              toolCallId: "tool",
              title: "Work",
              status: "completed",
              _meta: meta,
            },
          },
          { type: "turn_ended", stopReason: "end_turn" },
          { type: "completed" },
        ]),
      setConfig: async () => ({ configOptions: [] }),
      respondToPermission: async () => ({}),
    };
    const store = createChatStore(client);
    await store.getState().sendMessage({ oraSessionId: "live", text: "go" });
    const liveItem = store.getState().conversations.live!.turns[0]!.items[0]!;
    await store.getState().loadSession("live");
    const cold = createChatStore(client);
    await cold.getState().loadSession("cold");
    for (const item of [
      liveItem,
      store.getState().conversations.live!.turns[0]!.items[0]!,
      cold.getState().conversations.cold!.turns[0]!.items[0]!,
    ]) {
      assert.equal(item.kind, "toolCall");
      if (item.kind === "toolCall") assert.equal(item.status, "completed");
    }
    store.getState().dispose();
    cold.getState().dispose();
  }
});

test("a source switch does not discard leading whitespace or merge unidentified runs", async () => {
  const updates = [text("left", " "), text("right", "B"), text("left", "A")];
  const client: ChatSessionClient = {
    prompt: () =>
      events<PromptSessionEvent>([
        ...updates.map((update) => ({
          type: "session_update" as const,
          update,
        })),
        { type: "completed", stopReason: "end_turn" },
      ]),
    load: () => events<LoadSessionEvent>([]),
    setConfig: async () => ({ configOptions: [] }),
    respondToPermission: async () => ({}),
  };
  const store = createChatStore(client);
  await store.getState().sendMessage({ oraSessionId: "one", text: "go" });
  const items = store.getState().conversations.one!.turns[0]!.items;
  assert.deepEqual(
    items.map((item) => (item.kind === "message" ? item.content : null)),
    [" ", "B", "A"],
  );
  store.getState().dispose();
});

test("replaces _subagent/update plans by child source id across changing extra metadata in live and reload", async () => {
  const plan = (taskId: string, step: string, rev: number) => ({
    sessionUpdate: "plan" as const,
    entries: [
      {
        content: step,
        priority: "medium" as const,
        status: "pending" as const,
      },
    ],
    _meta: {
      "_subagent/update": {
        sessionId: "main",
        taskId,
        toolCallId: `parent-${taskId}`,
      },
      codeAgentCLI: { rev },
    },
  });
  const liveUpdates = [
    plan("task-a", "Step 1", 1),
    plan("task-b", "Sibling", 1),
    plan("task-a", "Step 2", 2),
  ];
  const client: ChatSessionClient = {
    prompt: () =>
      events<PromptSessionEvent>([
        ...liveUpdates.map((update) => ({
          type: "session_update" as const,
          update,
        })),
        { type: "completed", stopReason: "end_turn" },
      ]),
    load: () =>
      events<LoadSessionEvent>([
        {
          type: "session_update",
          update: {
            sessionUpdate: "user_message_chunk",
            content: { type: "text", text: "go" },
          },
        },
        { type: "session_update", update: plan("task-a", "Step 2", 2) },
        { type: "session_update", update: plan("task-b", "Sibling", 1) },
        { type: "turn_ended", stopReason: "end_turn" },
        { type: "completed" },
      ]),
    setConfig: async () => ({ configOptions: [] }),
    respondToPermission: async () => ({}),
  };
  const store = createChatStore(client);
  await store.getState().sendMessage({ oraSessionId: "live", text: "go" });
  const cold = createChatStore(client);
  await cold.getState().loadSession("cold");
  const summary = (items: ChatTurnItem[]) =>
    items.map((item) =>
      item.kind === "plan"
        ? {
            sourceId: childSessionSource(item.metadata)?.id,
            entries: item.entries.map((entry) => entry.content),
          }
        : null,
    );
  const expected = [
    { sourceId: "task-a", entries: ["Step 2"] },
    { sourceId: "task-b", entries: ["Sibling"] },
  ];
  assert.deepEqual(
    summary(store.getState().conversations.live!.turns[0]!.items),
    expected,
  );
  assert.deepEqual(
    summary(cold.getState().conversations.cold!.turns[0]!.items),
    expected,
  );
  store.getState().dispose();
  cold.getState().dispose();
});
