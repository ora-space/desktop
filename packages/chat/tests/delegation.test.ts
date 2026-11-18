import assert from "node:assert/strict";
import test from "node:test";
import type { ChatToolCall } from "../src/index.ts";
import {
  childSessionSource,
  delegationResultCompleted,
  delegationSessionId,
  delegationToolCallCompleted,
} from "../src/index.ts";

/** Builds the provider's delegation result shape as the store keeps it. */
function delegation(
  sessionID: unknown,
  status: unknown,
  toolStatus: ChatToolCall["status"],
): ChatToolCall {
  return {
    kind: "toolCall",
    id: "delegate",
    title: "subagent",
    toolKind: "think",
    status: toolStatus,
    content: [],
    locations: [],
    rawOutput: { metadata: { sessionID, status } },
    createdAt: 1,
    updatedAt: 2,
  };
}

test("reads only the provider's session id from the delegation result", () => {
  assert.equal(
    delegationSessionId(delegation("ses_child", "completed", "completed")),
    "ses_child",
  );
  assert.equal(
    delegationSessionId(delegation("  ", "completed", "completed")),
    undefined,
  );
  assert.equal(
    delegationSessionId(delegation(42, "completed", "completed")),
    undefined,
  );
  assert.equal(
    delegationSessionId({
      ...delegation("ses_child", "completed", "completed"),
      rawOutput: undefined,
    }),
    undefined,
  );
  assert.equal(
    delegationSessionId({
      ...delegation("ses_child", "completed", "completed"),
      rawOutput: { metadata: { sessionId: "ses_child" } },
    }),
    undefined,
  );
});

test("requires both the ACP completion and the provider's completed result", () => {
  assert.equal(
    delegationResultCompleted(
      delegation("ses_child", "completed", "completed"),
    ),
    true,
  );
  // The provider returns a background delegation while the worker still runs.
  assert.equal(
    delegationResultCompleted(delegation("ses_child", "running", "completed")),
    false,
  );
  // The chat store settles a parent tool the turn ended on, so a missing raw result cannot count.
  assert.equal(
    delegationResultCompleted(delegation(undefined, undefined, "completed")),
    false,
  );
  assert.equal(
    delegationResultCompleted(delegation("ses_child", "completed", "failed")),
    false,
  );
  assert.equal(
    delegationResultCompleted(delegation("ses_child", "completed", "unknown")),
    false,
  );
  assert.equal(
    delegationResultCompleted({
      ...delegation("ses_child", "completed", "completed"),
      rawOutput: { output: "done" },
    }),
    false,
  );
});

test("parses _subagent/update attribution and validates toolCallId delegation completion", () => {
  assert.deepEqual(
    childSessionSource({
      "_subagent/update": {
        sessionId: "ses_main",
        taskId: "task_1",
        toolCallId: "parent_call_1",
        agentType: "Explore",
      },
    }),
    {
      id: "task_1",
      parentId: "ses_main",
      depth: 1,
      title: "Explore",
      delegationToolCallId: "parent_call_1",
    },
  );
  assert.equal(
    childSessionSource({ "_subagent/update": { sessionId: " ", taskId: "t" } }),
    undefined,
  );
  assert.equal(
    childSessionSource({ "_subagent/update": { sessionId: "s", taskId: " " } }),
    undefined,
  );

  // Completed parent Agent/Task call with string rawOutput or content counts as returned.
  assert.equal(
    delegationToolCallCompleted({
      ...delegation(undefined, undefined, "completed"),
      rawOutput: "Subagent finished exploring",
    }),
    true,
  );
  // Turn-end synthesized completion (no content and undefined rawOutput) does not count.
  assert.equal(
    delegationToolCallCompleted({
      ...delegation(undefined, undefined, "completed"),
      rawOutput: undefined,
    }),
    false,
  );
  assert.equal(
    delegationToolCallCompleted({
      ...delegation(undefined, undefined, "failed"),
      rawOutput: "error",
    }),
    false,
  );
});
