import assert from "node:assert/strict";
import test from "node:test";
import type * as acp from "@agentclientprotocol/sdk";
import type { PromptSessionEvent } from "@ora/contracts";
import { createChatStore, type ChatSessionClient } from "../src/index.ts";
import { observeDiagnostics } from "../src/diagnostics.ts";
import { childSessionSource } from "../src/item-metadata.ts";

const validSource = { id: "child", parentID: "root", depth: 1 };
const invalidSources = [
  null,
  {},
  { ...validSource, id: " \t" },
  { ...validSource, parentID: " \t" },
  ...[0, -1, 1.5, "1", Number.MAX_SAFE_INTEGER + 1].map((depth) => ({
    ...validSource,
    depth,
  })),
  [],
  "child",
];

test("diagnostic activity and provider reports use the canonical effective source rule", () => {
  for (const source of [
    ...invalidSources,
    validSource,
    { ...validSource, depth: 1.0 },
    { ...validSource, depth: Number.MAX_SAFE_INTEGER },
  ]) {
    const _meta = { "opencode/child-session": source };
    const child = childSessionSource(_meta) !== undefined;
    const updates: acp.SessionUpdate[] = [
      {
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: "message" },
        _meta,
      },
      {
        sessionUpdate: "agent_thought_chunk",
        content: { type: "text", text: "thought" },
        _meta,
      },
      { sessionUpdate: "plan", entries: [], _meta },
      {
        sessionUpdate: "tool_call",
        toolCallId: "tool",
        title: "tool",
        status: "in_progress",
        _meta,
      },
      {
        sessionUpdate: "tool_call_update",
        toolCallId: "tool",
        status: "in_progress",
        _meta,
      },
    ];
    for (const update of updates) {
      const observed = observeDiagnostics(undefined, update, 100)!;
      assert.equal(observed.lastActivity?.receivedAt, child ? undefined : 100);
      assert.equal(
        observed.lastChildActivity?.receivedAt,
        child ? 100 : undefined,
      );
      if ("toolCallId" in update)
        assert.deepEqual(observed.toolSources, {
          tool: child ? "child" : "parent",
        });
    }
    for (const report of [
      {
        metadata: { "opencode/retry": { attempt: 2 } },
        provider: { kind: "retry", attempt: 2 },
      },
      {
        metadata: { "opencode/compaction": { status: "started" } },
        provider: { kind: "compaction", status: "started" },
      },
    ]) {
      assert.deepEqual(
        observeDiagnostics(
          undefined,
          {
            sessionUpdate: "session_info_update",
            _meta: { ..._meta, ...report.metadata },
          },
          200,
        ),
        child ? undefined : { provider: report.provider },
      );
    }
  }
});

test("explicit null and invalid sources clear child inheritance in a live attempt", async () => {
  for (const source of invalidSources) {
    let time = 100;
    const client: ChatSessionClient = {
      async *prompt() {
        yield {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "tool",
            title: "Tool",
            status: "in_progress",
            _meta: { "opencode/child-session": validSource },
          },
        };
        const conversation = () => store.getState().conversations.one!;
        assert.deepEqual(conversation().diagnostics, {
          lastChildActivity: { kind: "tool", receivedAt: 100 },
          toolSources: { tool: "child" },
        });
        time = 200;
        yield {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call_update",
            toolCallId: "tool",
            _meta: { "opencode/child-session": source },
          },
        };
        assert.deepEqual(conversation().diagnostics, {
          lastChildActivity: { kind: "tool", receivedAt: 100 },
          lastActivity: { kind: "tool", receivedAt: 200 },
          toolSources: { tool: "parent" },
        });
        const tool = conversation()
          .turns.at(-1)!
          .items.find((item) => item.kind === "toolCall")!;
        assert.equal(childSessionSource(tool.metadata), undefined);
        time = 300;
        // Other namespaces must not prevent inheritance of the now-parent source.
        yield {
          type: "session_update",
          update: {
            sessionUpdate: "tool_call_update",
            toolCallId: "tool",
            _meta: { unrelated: true },
          },
        };
        time = 400;
        yield {
          type: "session_update",
          update: { sessionUpdate: "tool_call_update", toolCallId: "tool" },
        };
        assert.deepEqual(conversation().diagnostics, {
          lastChildActivity: { kind: "tool", receivedAt: 100 },
          lastActivity: { kind: "tool", receivedAt: 400 },
          toolSources: { tool: "parent" },
        });
        yield { type: "completed", stopReason: "end_turn" };
      },
      async *load() {
        yield { type: "completed" };
      },
      getOptions: async () => ({ configOptions: null }),
      setConfig: async () => ({ configOptions: [] }),
      respondToPermission: async () => ({}),
    };
    const store = createChatStore(client, { now: () => time });
    try {
      await store.getState().sendMessage({ oraSessionId: "one", text: "go" });
    } finally {
      store.getState().dispose();
    }
  }
});

test("only recognized provider reports establish state and null clears retries", () => {
  assert.equal(
    observeDiagnostics(
      undefined,
      { sessionUpdate: "session_info_update", _meta: { status: "healthy" } },
      100,
    ),
    undefined,
  );
  const retry = observeDiagnostics(
    undefined,
    {
      sessionUpdate: "session_info_update",
      _meta: { "opencode/retry": { attempt: 2 } },
    },
    100,
  );
  assert.deepEqual(retry, { provider: { kind: "retry", attempt: 2 } });
  assert.deepEqual(
    observeDiagnostics(
      retry,
      {
        sessionUpdate: "session_info_update",
        _meta: { "opencode/retry": null },
      },
      101,
    ),
    { provider: undefined },
  );
});

test("live observation clears on reload and remains isolated from another session", async () => {
  const client: ChatSessionClient = {
    async *prompt() {
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "agent_message_chunk",
          content: { type: "text", text: "hello" },
        },
      } satisfies PromptSessionEvent;
      yield {
        type: "completed",
        stopReason: "end_turn",
      } satisfies PromptSessionEvent;
    },
    async *load() {
      yield {
        type: "session_update" as const,
        update: {
          sessionUpdate: "agent_message_chunk" as const,
          content: { type: "text" as const, text: "old reply" },
        },
        recordedAt: "2026-10-19T12:00:00+08:00",
      };
      yield { type: "completed" as const };
    },
    getOptions: async () => ({ configOptions: null }),
    setConfig: async () => ({ configOptions: [] }),
    respondToPermission: async () => ({}),
  };
  const store = createChatStore(client, { now: () => 123456 });
  store.getState().initializeSession("other");
  await store.getState().sendMessage({ oraSessionId: "one", text: "hi" });
  assert.deepEqual(store.getState().conversations.one?.diagnostics, {
    lastActivity: { kind: "message", receivedAt: 123456 },
  });
  assert.equal(store.getState().conversations.other?.diagnostics, undefined);
  await store.getState().loadSession("one");
  assert.equal(store.getState().conversations.one?.diagnostics, undefined);
  store.getState().dispose();
});

test("projected text and metadata-free child tool deltas never advance the parent observation", async () => {
  let time = 100;
  let markReady = () => {};
  let finish = () => {};
  const ready = new Promise<void>((resolve) => {
    markReady = resolve;
  });
  const completed = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const childMeta = {
    "opencode/child-session": { id: "child", parentID: "root", depth: 1 },
  };
  const client: ChatSessionClient = {
    async *prompt() {
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "agent_message_chunk",
          content: { type: "text", text: "Parent" },
        },
      } satisfies PromptSessionEvent;
      time = 200;
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "agent_message_chunk",
          content: { type: "text", text: "Background" },
          _meta: childMeta,
        },
      } satisfies PromptSessionEvent;
      time = 300;
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "tool_call",
          toolCallId: "child-tool",
          title: "Child tool",
          status: "in_progress",
          _meta: childMeta,
        },
      } satisfies PromptSessionEvent;
      time = 400;
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "tool_call_update",
          toolCallId: "child-tool",
          status: "in_progress",
        },
      } satisfies PromptSessionEvent;
      markReady();
      await completed;
      yield {
        type: "retrying",
        retry: 1,
        maxRetries: 3,
      } satisfies PromptSessionEvent;
      time = 500;
      yield {
        type: "session_update",
        update: {
          sessionUpdate: "tool_call",
          toolCallId: "child-tool",
          title: "Reused by parent",
          status: "in_progress",
        },
      } satisfies PromptSessionEvent;
      yield {
        type: "completed",
        stopReason: "end_turn",
      } satisfies PromptSessionEvent;
    },
    async *load() {
      yield { type: "completed" as const };
    },
    getOptions: async () => ({ configOptions: null }),
    setConfig: async () => ({ configOptions: [] }),
    respondToPermission: async () => ({}),
  };
  const store = createChatStore(client, { now: () => time });
  const sending = store
    .getState()
    .sendMessage({ oraSessionId: "one", text: "go" });
  await ready;
  assert.deepEqual(store.getState().conversations.one!.diagnostics, {
    lastActivity: { kind: "message", receivedAt: 100 },
    lastChildActivity: { kind: "tool", receivedAt: 400 },
    toolSources: { "child-tool": "child" },
  });
  finish();
  await sending;
  assert.deepEqual(store.getState().conversations.one!.diagnostics, {
    lastActivity: { kind: "tool", receivedAt: 500 },
    toolSources: { "child-tool": "parent" },
  });
  store.getState().adoptSwitchedAgent("one", []);
  assert.equal(store.getState().conversations.one!.diagnostics, undefined);
  store.getState().dispose();
});

test("failure and cancellation clear transient reports while a new prompt clears prior observations", async () => {
  for (const ending of ["failure", "cancel"] as const) {
    let first = true;
    const client: ChatSessionClient = {
      async *prompt() {
        if (first) {
          first = false;
          yield {
            type: "session_update",
            update: {
              sessionUpdate: "agent_message_chunk",
              content: { type: "text", text: "Observed" },
            },
          } satisfies PromptSessionEvent;
          yield {
            type: "session_update",
            update: {
              sessionUpdate: "session_info_update",
              _meta: { "opencode/retry": { attempt: 1 } },
            },
          } satisfies PromptSessionEvent;
          if (ending === "failure") throw new Error("transport lost");
          yield {
            type: "completed",
            stopReason: "cancelled",
          } satisfies PromptSessionEvent;
        } else
          yield {
            type: "completed",
            stopReason: "end_turn",
          } satisfies PromptSessionEvent;
      },
      async *load() {
        yield { type: "completed" as const };
      },
      getOptions: async () => ({ configOptions: null }),
      setConfig: async () => ({ configOptions: [] }),
      respondToPermission: async () => ({}),
    };
    const store = createChatStore(client, { now: () => 50 });
    const sending = store
      .getState()
      .sendMessage({ oraSessionId: "one", text: "go" });
    if (ending === "failure") await assert.rejects(sending, /transport lost/);
    else await sending;
    assert.equal(store.getState().conversations.one!.isResponding, false);
    assert.equal(
      store.getState().conversations.one!.diagnostics?.provider,
      undefined,
    );
    assert.deepEqual(
      store.getState().conversations.one!.diagnostics?.lastActivity,
      { kind: "message", receivedAt: 50 },
    );
    await store.getState().sendMessage({ oraSessionId: "one", text: "again" });
    // A streamed prompt starts an empty live snapshot, so the panel can say that
    // nothing was observed yet instead of reusing the replay wording.
    assert.deepEqual(store.getState().conversations.one!.diagnostics, {});
    store.getState().dispose();
  }
});

test("a load stream that replays an open turn exposes no live observation", async () => {
  let release = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let previewPublished = () => {};
  const preview = new Promise<void>((resolve) => {
    previewPublished = resolve;
  });
  const client: ChatSessionClient = {
    async *prompt() {
      yield {
        type: "completed",
        stopReason: "end_turn",
      } satisfies PromptSessionEvent;
    },
    async *load() {
      yield {
        type: "session_update" as const,
        update: {
          sessionUpdate: "tool_call" as const,
          toolCallId: "tool",
          title: "working",
          status: "in_progress" as const,
        },
        recordedAt: "2026-10-19T12:00:00+08:00",
      };
      previewPublished();
      await gate;
      yield { type: "completed" as const };
    },
    getOptions: async () => ({ configOptions: null }),
    setConfig: async () => ({ configOptions: [] }),
    respondToPermission: async () => ({}),
  };
  const store = createChatStore(client, { now: () => 123456 });
  const loading = store.getState().loadSession("one");
  await preview;
  const streaming = store.getState().conversations.one;
  assert.equal(streaming?.isLoading, true);
  assert.equal(streaming?.isResponding, true);
  // The replay is visibly still streaming, but this client did not observe it
  // live: an absent snapshot keeps the panel on the unknown wording.
  assert.equal(streaming?.diagnostics, undefined);
  release();
  await loading;
  const loaded = store.getState().conversations.one;
  assert.equal(loaded?.isLoaded, true);
  assert.equal(loaded?.isResponding, false);
  assert.equal(loaded?.diagnostics, undefined);
  store.getState().dispose();
});

test("_subagent/update tool calls record child activity and inherit across metadata-free deltas", () => {
  const subagentMeta = {
    "_subagent/update": {
      sessionId: "ses-main",
      taskId: "task-1",
      toolCallId: "parent-call",
      agentType: "Explore",
    },
  };
  const first = observeDiagnostics(
    undefined,
    {
      sessionUpdate: "tool_call",
      toolCallId: "task-1:inner-1",
      title: "Read",
      status: "pending",
      _meta: subagentMeta,
    },
    100,
  );
  assert.deepEqual(first, {
    lastChildActivity: { kind: "tool", receivedAt: 100 },
    toolSources: { "task-1:inner-1": "child" },
  });

  const delta = observeDiagnostics(
    first,
    {
      sessionUpdate: "tool_call_update",
      toolCallId: "task-1:inner-1",
      status: "completed",
    },
    200,
  );
  assert.deepEqual(delta, {
    lastChildActivity: { kind: "tool", receivedAt: 200 },
    toolSources: { "task-1:inner-1": "child" },
  });

  const cleared = observeDiagnostics(
    delta,
    {
      sessionUpdate: "tool_call_update",
      toolCallId: "task-1:inner-1",
      _meta: { "_subagent/update": null },
    },
    300,
  );
  assert.deepEqual(cleared, {
    lastChildActivity: { kind: "tool", receivedAt: 200 },
    lastActivity: { kind: "tool", receivedAt: 300 },
    toolSources: { "task-1:inner-1": "parent" },
  });
});
