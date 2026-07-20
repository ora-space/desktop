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
