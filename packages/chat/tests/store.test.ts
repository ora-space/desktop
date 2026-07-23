import assert from "node:assert/strict";
import test from "node:test";
import type { LoadSessionEvent, PromptSessionEvent } from "@ora/contracts";
import { createChatStore, type ChatSessionClient } from "../src/index.js";

/** Builds one ACP text update without exposing protocol transport details to the tests. */
function textEvent(
  role: "user_message_chunk" | "agent_message_chunk",
  text: string,
  messageId: string,
): LoadSessionEvent {
  return {
    type: "session_update",
    update: {
      sessionUpdate: role,
      messageId,
      content: { type: "text", text },
    },
  };
}

/** Yields a deterministic finite stream in the same shape as the generated client. */
async function* events<Event>(items: Event[]): AsyncIterable<Event> {
  for (const item of items) yield item;
}

test("loads provider history and reconstructs turns from message boundaries", async () => {
  const client: ChatSessionClient = {
    load: () => events([
      textEvent("user_message_chunk", "hel", "user-1"),
      textEvent("user_message_chunk", "lo", "user-1"),
      textEvent("user_message_chunk", "again", "user-2"),
      textEvent("agent_message_chunk", "hi", "agent-1"),
      { type: "completed" },
    ]),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
  };
  let nextId = 0;
  const store = createChatStore(client, {
    createId: () => `local-${++nextId}`,
    now: () => 42,
  });

  await store.getState().loadSession("ora-1");

  assert.deepEqual(store.getState().conversations["ora-1"], {
    turns: [
      {
        id: "local-1",
        userMessage: { kind: "message", id: "local-2", role: "user", content: "hello", createdAt: 42, protocolMessageId: "user-1" },
        items: [],
        status: "completed",
        stopReason: null,
        error: null,
        createdAt: 42,
      },
      {
        id: "local-3",
        userMessage: { kind: "message", id: "local-4", role: "user", content: "again", createdAt: 42, protocolMessageId: "user-2" },
        items: [
          { kind: "message", id: "message-agent-1", role: "assistant", content: "hi", createdAt: 42, protocolMessageId: "agent-1" },
        ],
        status: "completed",
        stopReason: null,
        error: null,
        createdAt: 42,
      },
    ],
    isLoaded: true,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    error: null,
  });
});

test("aborting a prompt retains the partial response and marks the turn cancelled", async () => {
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: (_request, options) => ({
      async *[Symbol.asyncIterator]() {
        yield textEvent("agent_message_chunk", "partial", "agent-1") as PromptSessionEvent;
        yield {
          type: "permission_request",
          permissionRequestId: "permission-1",
          toolCall: { toolCallId: "tool-1", title: "Run command" },
          options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
        } satisfies PromptSessionEvent;
        await new Promise<void>((_resolve, reject) => {
          options?.signal?.addEventListener("abort", () => {
            const error = new Error("cancelled");
            error.name = "AbortError";
            reject(error);
          }, { once: true });
        });
      },
    }),
    respondToPermission: async () => ({}),
  };
  const store = createChatStore(client, { createId: () => "id-1", now: () => 42 });
  const sending = store.getState().sendMessage({ oraSessionId: "ora-1", text: " hello " });
  await new Promise<void>((resolve) => setTimeout(resolve, 0));

  store.getState().stopGeneration("ora-1");
  await sending;

  const conversation = store.getState().conversations["ora-1"];
  assert.deepEqual(conversation?.turns, [
    {
      id: "id-1",
      userMessage: { kind: "message", id: "id-1", role: "user", content: "hello", createdAt: 42 },
      items: [
        { kind: "message", id: "message-agent-1", role: "assistant", content: "partial", createdAt: 42, protocolMessageId: "agent-1" },
      ],
      status: "cancelled",
      stopReason: null,
      error: null,
      createdAt: 42,
    },
  ]);
  assert.equal(conversation?.isResponding, false);
  assert.deepEqual(conversation?.pendingPermissions, []);
});

test("shows the user turn on a draft key before promoting to the created session", async () => {
  let promptSessionId: string | undefined;
  const client: ChatSessionClient = {
    load: () => events<LoadSessionEvent>([]),
    prompt: (request) => {
      promptSessionId = request.sessionId;
      return events<PromptSessionEvent>([
        textEvent("agent_message_chunk", "done", "agent-1") as PromptSessionEvent,
        { type: "completed", stopReason: "end_turn" },
      ]);
    },
    respondToPermission: async () => ({}),
  };
  let nextId = 0;
  const store = createChatStore(client, { createId: () => `local-${++nextId}`, now: () => 42 });

  let resolveCreate: (id: string) => void = () => {};
  const created = new Promise<string>((resolve) => { resolveCreate = resolve; });
  const drafts: string[] = [];
  const promoted: string[] = [];

  const sending = store.getState().sendMessage({
    text: "hi",
    createSession: () => created,
    onDraft: (id) => drafts.push(id),
    onSessionCreated: (id) => promoted.push(id),
  });

  // The turn is visible under the draft key while the session is still being created.
  assert.deepEqual(drafts, ["draft-local-1"]);
  const draft = store.getState().conversations["draft-local-1"];
  assert.equal(draft?.turns.length, 1);
  assert.equal(draft?.turns[0]?.userMessage.content, "hi");
  assert.equal(draft?.isResponding, true);
  assert.equal(store.getState().conversations["real-session"], undefined);

  resolveCreate("real-session");
  await sending;

  // The conversation has moved onto the real id and the draft key is gone.
  assert.deepEqual(promoted, ["real-session"]);
  assert.equal(promptSessionId, "real-session");
  assert.equal(store.getState().conversations["draft-local-1"], undefined);
  const conversation = store.getState().conversations["real-session"];
  assert.equal(conversation?.isResponding, false);
  // The live turn is authoritative, so the promoted conversation is already
  // "loaded" and the workspace never re-loads (and re-slides) it.
  assert.equal(conversation?.isLoaded, true);
  assert.deepEqual(conversation?.turns[0]?.items, [
    { kind: "message", id: "message-agent-1", role: "assistant", content: "done", createdAt: 42, protocolMessageId: "agent-1" },
  ]);
  assert.equal(conversation?.turns[0]?.status, "completed");
});

test("rolls back staged load updates when replay fails before completion", async () => {
  const client: ChatSessionClient = {
    load: () => ({
      async *[Symbol.asyncIterator]() {
        yield textEvent("agent_message_chunk", "uncommitted", "agent-new");
        throw new Error("load failed");
      },
    }),
    prompt: () => events<PromptSessionEvent>([]),
    respondToPermission: async () => ({}),
  };
  const store = createChatStore(client, { createId: () => "local", now: () => 42 });
  const previousTurn = {
    id: "old-turn",
    userMessage: { kind: "message" as const, id: "old-user", role: "user" as const, content: "prompt", createdAt: 1 },
    items: [
      { kind: "message" as const, id: "old", role: "assistant" as const, content: "history", createdAt: 1 },
    ],
    status: "completed" as const,
    stopReason: null,
    error: null,
    createdAt: 1,
  };
  store.setState({
    conversations: {
      "ora-1": {
        turns: [previousTurn],
        isLoaded: true,
        isLoading: false,
        isResponding: false,
        pendingPermissions: [],
        error: null,
      },
    },
  });

  await assert.rejects(store.getState().loadSession("ora-1"), /load failed/);

  assert.deepEqual(store.getState().conversations["ora-1"], {
    turns: [previousTurn],
    isLoaded: true,
    isLoading: false,
    isResponding: false,
    pendingPermissions: [],
    error: "load failed",
  });
});
