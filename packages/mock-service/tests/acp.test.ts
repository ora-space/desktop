import assert from "node:assert/strict";
import test from "node:test";
import type { acp } from "@ora/contracts";
import { createChatStore, exerciseAcpClientConformance } from "@ora/chat";
import { createMockAcpClient } from "../src/acp.js";
import { defaultMockAcpScenarioResolver } from "../src/acp-scenario.js";
import { mockChatSuggestions } from "../src/chat-suggestions.js";

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

test("routes bilingual operation and failure prompts deterministically", () => {
  assert.equal(defaultMockAcpScenarioResolver("请修改 greeting 实现"), "tool_success");
  assert.equal(defaultMockAcpScenarioResolver("Fix the greeting"), "tool_success");
  assert.equal(defaultMockAcpScenarioResolver("修改不存在的文件"), "tool_failure");
  assert.equal(defaultMockAcpScenarioResolver("Explain the greeting"), "chat");
  assert.equal(defaultMockAcpScenarioResolver("总结 Agent Runtime 重构进展"), "chat");
  assert.equal(defaultMockAcpScenarioResolver("Summarize the agent runtime refactor"), "chat");
  assert.equal(defaultMockAcpScenarioResolver("请帮我解释如何修改 greeting"), "chat");
});

test("emits grouped reads, edits, commands, and final text for an operation prompt", async () => {
  let id = 0;
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    createId: () => `id-${++id}`,
    scenarioResolver: () => "tool_success",
  });
  const notifications: acp.SessionNotification[] = [];
  client.subscribe((notification) => notifications.push(notification));

  const response = await client.prompt({
    sessionId: "agent-session-runtime",
    prompt: [{ type: "text", text: "implement and test greeting" }],
  });

  assert.deepEqual(response, { stopReason: "end_turn" });
  assert.equal(notifications.some((notification) => notification.update.sessionUpdate === "agent_thought_chunk"), true);
  assert.equal(notifications.filter((notification) => notification.update.sessionUpdate === "plan").length, 6);
  assert.deepEqual(
    notifications
      .filter((notification) => notification.update.sessionUpdate === "tool_call")
      .map((notification) => notification.update.sessionUpdate === "tool_call" ? notification.update.kind : undefined),
    ["read", "read", "edit", "edit", "execute", "execute"],
  );
  const editCompletions = notifications.filter((notification) =>
    notification.update.sessionUpdate === "tool_call_update"
    && notification.update.status === "completed"
    && notification.update.content?.some((content) => content.type === "diff"),
  );
  assert.equal(editCompletions.length, 2);
  const commandCompletions = notifications.filter((notification) =>
    notification.update.sessionUpdate === "tool_call_update"
    && notification.update.status === "completed"
    && notification.update.rawOutput !== undefined
    && "exitCode" in (notification.update.rawOutput as object),
  );
  assert.equal(commandCompletions.length, 2);
  const finalPlan = notifications
    .filter((notification) => notification.update.sessionUpdate === "plan")
    .at(-1);
  assert.equal(finalPlan?.update.sessionUpdate, "plan");
  if (finalPlan?.update.sessionUpdate === "plan") {
    assert.equal(finalPlan.update.entries.every((entry) => entry.status === "completed"), true);
  }
  assert.equal(
    notifications
      .filter((notification) => notification.update.sessionUpdate === "agent_message_chunk")
      .map(readText)
      .join(""),
    "Updated the greeting implementation and tests, then completed type checking and test verification.",
  );
});

test("persists virtual edits within one session without touching other sessions", async () => {
  let id = 0;
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    createId: () => `id-${++id}`,
    scenarioResolver: () => "tool_success",
  });
  const notifications: acp.SessionNotification[] = [];
  client.subscribe((notification) => notifications.push(notification));

  await client.prompt({
    sessionId: "agent-session-runtime",
    prompt: [{ type: "text", text: "implement greeting" }],
  });
  notifications.length = 0;
  await client.prompt({
    sessionId: "agent-session-runtime",
    prompt: [{ type: "text", text: "implement greeting again" }],
  });

  const readCompletion = notifications.find((notification) =>
    notification.update.sessionUpdate === "tool_call_update"
    && notification.update.status === "completed"
    && notification.update.content?.some((content) =>
      content.type === "content"
      && content.content.type === "text"
      && content.content.text.includes("normalizedName"),
    ),
  );
  assert.notEqual(readCompletion, undefined);
  const testReadCompletion = notifications.find((notification) =>
    notification.update.sessionUpdate === "tool_call_update"
    && notification.update.status === "completed"
    && notification.update.content?.some((content) =>
      content.type === "content"
      && content.content.type === "text"
      && content.content.text.includes("normalizes surrounding whitespace"),
    ),
  );
  assert.notEqual(testReadCompletion, undefined);
});

test("keeps injected starter prompts aligned with deterministic scenarios", () => {
  assert.deepEqual(
    mockChatSuggestions.map((suggestion) => defaultMockAcpScenarioResolver(suggestion.text["en-US"])),
    ["chat", "tool_success", "tool_failure"],
  );
  assert.deepEqual(
    mockChatSuggestions.map((suggestion) => defaultMockAcpScenarioResolver(suggestion.text["zh-CN"])),
    ["chat", "tool_success", "tool_failure"],
  );
});

test("keeps a failed tool inside a normally completed ACP turn", async () => {
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    createId: () => "failure",
    scenarioResolver: () => "tool_failure",
  });
  const notifications: acp.SessionNotification[] = [];
  client.subscribe((notification) => notifications.push(notification));

  const response = await client.prompt({
    sessionId: "agent-session-runtime",
    prompt: [{ type: "text", text: "modify a missing file" }],
  });

  assert.deepEqual(response, { stopReason: "end_turn" });
  assert.equal(
    notifications.some((notification) =>
      notification.update.sessionUpdate === "tool_call_update"
      && notification.update.status === "failed"),
    true,
  );
});

test("cancels an active tool and returns the ACP cancelled stop reason", async () => {
  const notifications: acp.SessionNotification[] = [];
  const scheduler = {
    wait: async () => {
      if (notifications.some((notification) => notification.update.sessionUpdate === "tool_call")) {
        await new Promise<void>(() => undefined);
      }
    },
  };
  const client = createMockAcpClient({
    scheduler,
    createId: () => "cancel",
    scenarioResolver: () => "tool_success",
  });
  client.subscribe((notification) => notifications.push(notification));

  const prompting = client.prompt({
    sessionId: "agent-session-runtime",
    prompt: [{ type: "text", text: "implement greeting" }],
  });
  await waitForNotification(notifications, "tool_call");
  await client.cancel({ sessionId: "agent-session-runtime" });
  const response = await prompting;

  assert.deepEqual(response, { stopReason: "cancelled" });
  assert.equal(
    notifications.some((notification) =>
      notification.update.sessionUpdate === "tool_call_update"
      && notification.update.status === "failed"
      && typeof notification.update.rawOutput === "object"),
    true,
  );
});

test("preserves the cancelled tool update in the shared chat state", async () => {
  const notifications: acp.SessionNotification[] = [];
  const client = createMockAcpClient({
    scheduler: {
      wait: async () => {
        if (notifications.some((notification) => notification.update.sessionUpdate === "tool_call")) {
          await new Promise<void>(() => undefined);
        }
      },
    },
    createId: () => "cancel-store",
    scenarioResolver: () => "tool_success",
  });
  client.subscribe((notification) => notifications.push(notification));
  const ids = ["turn-cancel", "user-cancel"];
  const store = createChatStore(client, { createId: () => ids.shift()! });
  const sending = store.getState().sendMessage({
    oraSessionId: "ora-cancel",
    agentSessionId: "agent-session-runtime",
    text: "implement greeting",
  });
  await waitForNotification(notifications, "tool_call");

  await store.getState().cancelMessage({
    oraSessionId: "ora-cancel",
    agentSessionId: "agent-session-runtime",
  });
  await sending;

  const turn = store.getState().conversations["ora-cancel"]!.turns[0]!;
  const tool = turn.items.find((item) => item.kind === "toolCall");
  assert.equal(turn.status, "cancelled");
  assert.equal(tool?.kind === "toolCall" ? tool.status : undefined, "failed");
});

test("passes the shared ACP client conformance exercise", async () => {
  let id = 0;
  const client = createMockAcpClient({
    scheduler: immediateScheduler,
    createId: () => `conformance-${++id}`,
    initialSessionIds: [],
    scenarioResolver: () => "tool_success",
  });

  const result = await exerciseAcpClientConformance(client, {
    newSessionRequest: { cwd: "/workspace/ora", mcpServers: [] },
    prompt: [{ type: "text", text: "implement greeting" }],
    cancelAfterFirstUpdate: true,
  });

  assert.equal(result.response.stopReason, "cancelled");
  assert.equal(result.notifications.length > 0, true);
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

/** Waits until the requested update type has been delivered by the async mock prompt. */
async function waitForNotification(
  notifications: acp.SessionNotification[],
  sessionUpdate: acp.SessionUpdate["sessionUpdate"],
): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (notifications.some((notification) => notification.update.sessionUpdate === sessionUpdate)) return;
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  throw new Error(`mock did not emit ${sessionUpdate}`);
}
