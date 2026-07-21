import assert from "node:assert/strict";
import test from "node:test";
import type { acp } from "@ora/contracts";
import {
  createChatStore,
  isConversationResponding,
  type AcpClient,
  type AcpSessionNotificationListener,
  type SessionConversation,
} from "../src/index.js";

class RecordingAcpClient implements AcpClient {
  readonly prompts: acp.PromptRequest[] = [];
  readonly cancellations: acp.CancelNotification[] = [];
  private readonly listeners = new Set<AcpSessionNotificationListener>();
  private readonly heldPrompts = new Map<string, (response: acp.PromptResponse) => void>();

  async newSession(_request: acp.NewSessionRequest): Promise<acp.NewSessionResponse> {
    return { sessionId: "agent-session-new" };
  }

  async prompt(request: acp.PromptRequest): Promise<acp.PromptResponse> {
    this.prompts.push(request);
    return new Promise((resolve) => {
      this.heldPrompts.set(request.sessionId, resolve);
    });
  }

  async cancel(notification: acp.CancelNotification): Promise<void> {
    this.cancellations.push(notification);
    this.release(notification.sessionId, "cancelled");
  }

  subscribe(listener: AcpSessionNotificationListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  emit(notification: acp.SessionNotification): void {
    this.listeners.forEach((listener) => listener(notification));
  }

  release(sessionId: string, stopReason: acp.StopReason = "end_turn"): void {
    this.heldPrompts.get(sessionId)?.({ stopReason });
    this.heldPrompts.delete(sessionId);
  }

  failPrompt(error: Error): void {
    this.prompt = async () => {
      throw error;
    };
  }
}

test("normalizes thought, plan, parallel tools, and message chunks into one turn", async () => {
  const client = new RecordingAcpClient();
  const ids = ["turn-1", "user-1"];
  const store = createChatStore(client, { createId: () => ids.shift()!, now: () => 100 });

  const sending = store.getState().sendMessage({
    oraSessionId: "ora-1",
    agentSessionId: "agent-1",
    text: " implement greeting ",
  });
  client.emit(textChunk("agent-1", "agent_thought_chunk", undefined, "Inspect "));
  client.emit(textChunk("agent-1", "agent_thought_chunk", undefined, "files"));
  client.emit({
    sessionId: "agent-1",
    update: {
      sessionUpdate: "plan",
      entries: [{ content: "Read", priority: "high", status: "in_progress" }],
    },
  });
  client.emit(toolCall("agent-1", "tool-read", "Read file", "read"));
  client.emit(toolCall("agent-1", "tool-edit", "Edit file", "edit"));
  client.emit({
    sessionId: "agent-1",
    update: {
      sessionUpdate: "tool_call_update",
      toolCallId: "tool-edit",
      status: "completed",
      rawOutput: { changed: true },
    },
  });
  client.emit({
    sessionId: "agent-1",
    update: {
      sessionUpdate: "plan",
      entries: [{ content: "Read", priority: "high", status: "completed" }],
    },
  });
  client.emit(textChunk("agent-1", "agent_message_chunk", "reply-1", "Done"));

  client.release("agent-1");
  await sending;

  assert.deepEqual(store.getState().conversations["ora-1"], {
    turns: [{
      id: "turn-1",
      userMessage: {
        kind: "message",
        id: "user-1",
        role: "user",
        content: "implement greeting",
        createdAt: 100,
      },
      items: [
        {
          kind: "thought",
          id: "thought-implicit-turn-1",
          content: "Inspect files",
          createdAt: 100,
        },
        {
          kind: "plan",
          id: "plan-turn-1",
          entries: [{ content: "Read", priority: "high", status: "completed" }],
          createdAt: 100,
          updatedAt: 100,
        },
        {
          kind: "toolCall",
          id: "tool-read",
          title: "Read file",
          toolKind: "read",
          status: "pending",
          content: [],
          locations: [],
          createdAt: 100,
          updatedAt: 100,
        },
        {
          kind: "toolCall",
          id: "tool-edit",
          title: "Edit file",
          toolKind: "edit",
          status: "completed",
          content: [],
          locations: [],
          rawOutput: { changed: true },
          createdAt: 100,
          updatedAt: 100,
        },
        {
          kind: "message",
          id: "message-reply-1",
          role: "assistant",
          content: "Done",
          createdAt: 100,
          protocolMessageId: "reply-1",
        },
      ],
      status: "completed",
      stopReason: "end_turn",
      error: null,
      createdAt: 100,
    }],
    error: null,
  });
  assert.equal(isConversationResponding(store.getState().conversations["ora-1"]), false);
});

test("routes concurrent streams to their owning sessions", async () => {
  const client = new RecordingAcpClient();
  let id = 0;
  const store = createChatStore(client, { createId: () => `id-${++id}` });
  const first = store.getState().sendMessage({ oraSessionId: "ora-1", agentSessionId: "agent-1", text: "first" });
  const second = store.getState().sendMessage({ oraSessionId: "ora-2", agentSessionId: "agent-2", text: "second" });

  client.emit(textChunk("agent-2", "agent_message_chunk", "reply-2", "two"));
  client.emit(textChunk("agent-1", "agent_message_chunk", "reply-1", "one"));
  client.release("agent-1");
  client.release("agent-2");
  await Promise.all([first, second]);

  assert.equal(readAssistantText(store.getState().conversations["ora-1"]!), "one");
  assert.equal(readAssistantText(store.getState().conversations["ora-2"]!), "two");
});

test("cancels only the active turn and records the protocol stop reason", async () => {
  const client = new RecordingAcpClient();
  const ids = ["turn-cancel", "user-cancel"];
  const store = createChatStore(client, { createId: () => ids.shift()! });
  const sending = store.getState().sendMessage({
    oraSessionId: "ora-cancel",
    agentSessionId: "agent-cancel",
    text: "implement",
  });

  await store.getState().cancelMessage({
    oraSessionId: "ora-cancel",
    agentSessionId: "agent-cancel",
  });
  await sending;

  assert.deepEqual(client.cancellations, [{ sessionId: "agent-cancel" }]);
  assert.equal(store.getState().conversations["ora-cancel"]!.turns[0]!.status, "cancelled");
  assert.equal(store.getState().conversations["ora-cancel"]!.turns[0]!.stopReason, "cancelled");
});

test("keeps unsupported content visible and accepts id-less agent chunks", async () => {
  const client = new RecordingAcpClient();
  const ids = ["turn-content", "user-content", "unsupported-1"];
  const store = createChatStore(client, { createId: () => ids.shift()!, now: () => 50 });
  const sending = store.getState().sendMessage({
    oraSessionId: "ora-content",
    agentSessionId: "agent-content",
    text: "show content",
  });

  client.emit(textChunk("agent-content", "agent_message_chunk", undefined, "Hello"));
  client.emit({
    sessionId: "agent-content",
    update: {
      sessionUpdate: "agent_message_chunk",
      content: { type: "image", data: "AA==", mimeType: "image/png" },
    },
  });
  client.release("agent-content");
  await sending;

  assert.deepEqual(store.getState().conversations["ora-content"]!.turns[0]!.items, [
    {
      kind: "message",
      id: "message-implicit-turn-content",
      role: "assistant",
      content: "Hello",
      createdAt: 50,
    },
    {
      kind: "unsupportedContent",
      id: "unsupported-1",
      source: "message",
      contentType: "image",
      createdAt: 50,
    },
  ]);
});

test("marks the turn failed while preserving the user message when prompting rejects", async () => {
  const client = new RecordingAcpClient();
  client.failPrompt(new Error("agent unavailable"));
  const ids = ["turn-error", "user-error"];
  const store = createChatStore(client, { createId: () => ids.shift()!, now: () => 200 });

  await assert.rejects(
    store.getState().sendMessage({
      oraSessionId: "ora-error",
      agentSessionId: "agent-error",
      text: "keep this",
    }),
    /agent unavailable/,
  );

  assert.deepEqual(store.getState().conversations["ora-error"], {
    turns: [{
      id: "turn-error",
      userMessage: {
        kind: "message",
        id: "user-error",
        role: "user",
        content: "keep this",
        createdAt: 200,
      },
      items: [],
      status: "failed",
      stopReason: null,
      error: "agent unavailable",
      createdAt: 200,
    }],
    error: "agent unavailable",
  });
});

/** Builds one text session update with either an explicit or implicit message id. */
function textChunk(
  sessionId: string,
  sessionUpdate: "agent_message_chunk" | "agent_thought_chunk",
  messageId: string | undefined,
  text: string,
): acp.SessionNotification {
  return {
    sessionId,
    update: {
      sessionUpdate,
      ...(messageId === undefined ? {} : { messageId }),
      content: { type: "text", text },
    },
  };
}

/** Builds one pending tool call notification. */
function toolCall(
  sessionId: string,
  toolCallId: string,
  title: string,
  kind: acp.ToolKind,
): acp.SessionNotification {
  return {
    sessionId,
    update: { sessionUpdate: "tool_call", toolCallId, title, kind, status: "pending" },
  };
}

/** Reads all assistant message text from the latest turn. */
function readAssistantText(conversation: SessionConversation): string {
  return conversation.turns.at(-1)!.items
    .filter((item) => item.kind === "message" && item.role === "assistant")
    .map((item) => item.kind === "message" ? item.content : "")
    .join("");
}
