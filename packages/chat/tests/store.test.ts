import assert from "node:assert/strict";
import test from "node:test";
import type { acp } from "@ora/contracts";
import {
  createChatStore,
  type AcpClient,
  type AcpSessionNotificationListener,
} from "../src/index.js";

class RecordingAcpClient implements AcpClient {
  readonly prompts: acp.PromptRequest[] = [];
  private readonly listeners = new Set<AcpSessionNotificationListener>();
  private promptCompletion: Promise<acp.PromptResponse> | null = null;

  async newSession(
    _request: acp.NewSessionRequest,
  ): Promise<acp.NewSessionResponse> {
    return { sessionId: "agent-session-new" };
  }

  async prompt(request: acp.PromptRequest): Promise<acp.PromptResponse> {
    this.prompts.push(request);
    return this.promptCompletion ?? { stopReason: "end_turn" };
  }

  subscribe(listener: AcpSessionNotificationListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  emit(notification: acp.SessionNotification): void {
    this.listeners.forEach((listener) => listener(notification));
  }

  holdPrompt(): () => void {
    let resolvePrompt!: (response: acp.PromptResponse) => void;
    this.promptCompletion = new Promise((resolve) => {
      resolvePrompt = resolve;
    });
    return () => {
      resolvePrompt({ stopReason: "end_turn" });
      this.promptCompletion = null;
    };
  }
}

test("sends text and merges streamed chunks by ACP message id", async () => {
  const client = new RecordingAcpClient();
  const releasePrompt = client.holdPrompt();
  const store = createChatStore(client, {
    createId: () => "user-message-1",
    now: () => 100,
  });

  const sending = store.getState().sendMessage({
    oraSessionId: "ora-session-1",
    agentSessionId: "agent-session-1",
    text: " Hello ",
  });
  client.emit(agentText("agent-session-1", "agent-message-1", "Mock "));
  client.emit(agentText("agent-session-1", "agent-message-1", "response"));

  assert.deepEqual(store.getState().conversations["ora-session-1"], {
    messages: [
      {
        id: "user-message-1",
        role: "user",
        content: "Hello",
        createdAt: 100,
      },
      {
        id: "agent-message-1",
        role: "assistant",
        content: "Mock response",
        createdAt: 100,
      },
    ],
    isResponding: true,
    error: null,
  });
  assert.deepEqual(client.prompts, [
    {
      sessionId: "agent-session-1",
      prompt: [{ type: "text", text: "Hello" }],
    },
  ]);

  releasePrompt();
  await sending;
  assert.equal(
    store.getState().conversations["ora-session-1"]!.isResponding,
    false,
  );
});

test("routes concurrent agent streams to their owning Ora sessions", async () => {
  const client = new RecordingAcpClient();
  const releasePrompt = client.holdPrompt();
  let id = 0;
  const store = createChatStore(client, { createId: () => `user-${++id}` });

  const first = store.getState().sendMessage({
    oraSessionId: "ora-1",
    agentSessionId: "agent-1",
    text: "first",
  });
  const second = store.getState().sendMessage({
    oraSessionId: "ora-2",
    agentSessionId: "agent-2",
    text: "second",
  });
  client.emit(agentText("agent-2", "reply-2", "two"));
  client.emit(agentText("agent-1", "reply-1", "one"));

  assert.equal(
    store.getState().conversations["ora-1"]!.messages[1]!.content,
    "one",
  );
  assert.equal(
    store.getState().conversations["ora-2"]!.messages[1]!.content,
    "two",
  );

  releasePrompt();
  await Promise.all([first, second]);
});

test("rejects a second prompt only within the same Ora session", async () => {
  const client = new RecordingAcpClient();
  const releasePrompt = client.holdPrompt();
  const store = createChatStore(client);
  const first = store.getState().sendMessage({
    oraSessionId: "ora-1",
    agentSessionId: "agent-1",
    text: "first",
  });

  await assert.rejects(
    store.getState().sendMessage({
      oraSessionId: "ora-1",
      agentSessionId: "agent-1",
      text: "second",
    }),
    /already processing/,
  );
  releasePrompt();
  await first;
});

test("keeps the user message and records an error when prompt processing fails", async () => {
  const client = new RecordingAcpClient();
  client.prompt = async () => {
    throw new Error("agent unavailable");
  };
  const store = createChatStore(client, {
    createId: () => "user-message-error",
    now: () => 200,
  });

  await assert.rejects(
    store.getState().sendMessage({
      oraSessionId: "ora-error",
      agentSessionId: "agent-error",
      text: "keep this",
    }),
    /agent unavailable/,
  );

  assert.deepEqual(store.getState().conversations["ora-error"], {
    messages: [
      {
        id: "user-message-error",
        role: "user",
        content: "keep this",
        createdAt: 200,
      },
    ],
    isResponding: false,
    error: "agent unavailable",
  });
});

/** Builds one valid text update from the mock agent. */
function agentText(
  sessionId: string,
  messageId: string,
  text: string,
): acp.SessionNotification {
  return {
    sessionId,
    update: {
      sessionUpdate: "agent_message_chunk",
      messageId,
      content: { type: "text", text },
    },
  };
}
