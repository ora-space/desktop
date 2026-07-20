import assert from "node:assert/strict";
import test from "node:test";
import type { acp } from "@ora/contracts";
import { createMockAcpClient } from "../src/acp.js";

const immediateScheduler = { wait: async () => undefined };

test("creates an ACP session and streams a deterministic reply with one message id", async () => {
  const ids = ["session-id", "message-id"];
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    chunkSize: 5,
    createId: () => ids.shift()!,
    initialSessionIds: [],
  });
  const notifications: acp.SessionNotification[] = [];
  client.subscribe((notification) => notifications.push(notification));

  const session = await client.newSession({
    cwd: "/workspace/ora",
    mcpServers: [],
  });
  const response = await client.prompt({
    sessionId: session.sessionId,
    prompt: [{ type: "text", text: "hello" }],
  });

  assert.deepEqual(session, { sessionId: "agent-session-session-id" });
  assert.deepEqual(response, { stopReason: "end_turn" });
  assert.equal(
    notifications.map(readText).join(""),
    "Mock response: hello",
  );
  assert.deepEqual(
    new Set(notifications.map(readMessageId)),
    new Set(["agent-message-message-id"]),
  );
});

test("supports the seeded agent session used by initial Ora mock data", async () => {
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    createId: () => "reply",
  });

  await assert.doesNotReject(
    client.prompt({
      sessionId: "agent-session-runtime",
      prompt: [{ type: "text", text: "existing session" }],
    }),
  );
});

test("rejects prompts for unknown agent sessions", async () => {
  const client = createMockAcpClient({ scheduler: immediateScheduler });

  await assert.rejects(
    client.prompt({
      sessionId: "missing",
      prompt: [{ type: "text", text: "hello" }],
    }),
    /ACP session not found/,
  );
});

test("reports the configured stop reason so callers cover every turn ending", async () => {
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    createId: () => "reply",
    stopReason: "refusal",
  });

  const response = await client.prompt({
    sessionId: "agent-session-runtime",
    prompt: [{ type: "text", text: "hello" }],
  });

  assert.deepEqual(response, { stopReason: "refusal" });
});

test("rejects without streaming when the agent is unreachable", async () => {
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    fault: { kind: "failBeforeStream", message: "agent unreachable" },
  });
  const notifications: acp.SessionNotification[] = [];
  client.subscribe((notification) => notifications.push(notification));

  await assert.rejects(
    client.prompt({
      sessionId: "agent-session-runtime",
      prompt: [{ type: "text", text: "hello" }],
    }),
    /agent unreachable/,
  );
  assert.deepEqual(notifications, []);
});

test("rejects mid-stream after delivering the chunks that already arrived", async () => {
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    chunkSize: 5,
    createId: () => "reply",
    fault: { kind: "failMidStream", afterChunks: 2, message: "stream dropped" },
  });
  const notifications: acp.SessionNotification[] = [];
  client.subscribe((notification) => notifications.push(notification));

  await assert.rejects(
    client.prompt({
      sessionId: "agent-session-runtime",
      prompt: [{ type: "text", text: "hello" }],
    }),
    /stream dropped/,
  );
  // The partial reply stays delivered: a dropped stream does not un-send text.
  assert.equal(notifications.map(readText).join(""), "Mock respo");
});

test("frees the session after a mid-stream failure so the next prompt is accepted", async () => {
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    createId: () => "reply",
    fault: { kind: "failMidStream", afterChunks: 1, message: "stream dropped" },
  });

  await assert.rejects(
    client.prompt({
      sessionId: "agent-session-runtime",
      prompt: [{ type: "text", text: "first" }],
    }),
    /stream dropped/,
  );
  await assert.rejects(
    client.prompt({
      sessionId: "agent-session-runtime",
      prompt: [{ type: "text", text: "second" }],
    }),
    /stream dropped/,
  );
});

test("streams the full reply but never settles when the turn stalls", async () => {
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    chunkSize: 5,
    createId: () => "reply",
    fault: { kind: "hang" },
  });
  const notifications: acp.SessionNotification[] = [];
  client.subscribe((notification) => notifications.push(notification));

  const prompting = client
    .prompt({
      sessionId: "agent-session-runtime",
      prompt: [{ type: "text", text: "hello" }],
    })
    .then(() => "settled");
  // Let every chunk drain before checking that the turn itself never completes.
  await new Promise((resolve) => setTimeout(resolve, 0));
  const settled = await Promise.race([prompting, Promise.resolve("pending")]);

  assert.equal(settled, "pending");
  assert.equal(notifications.map(readText).join(""), "Mock response: hello");
});

/** Reads text from the known agent text notification produced by the mock. */
function readText(notification: acp.SessionNotification): string {
  const update = notification.update;
  assert.equal(update.sessionUpdate, "agent_message_chunk");
  if (update.sessionUpdate !== "agent_message_chunk") return "";
  assert.equal(update.content.type, "text");
  return update.content.type === "text" ? update.content.text : "";
}

/** Reads the required message identifier from one mock agent chunk. */
function readMessageId(notification: acp.SessionNotification): string {
  const update = notification.update;
  assert.equal(update.sessionUpdate, "agent_message_chunk");
  if (update.sessionUpdate !== "agent_message_chunk") return "";
  assert.equal(typeof update.messageId, "string");
  return update.messageId!;
}
