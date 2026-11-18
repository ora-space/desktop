import { describe, expect, it } from "vitest";
import type { ChatToolCall, ChatTurnItem } from "@ora/chat";
import {
  delegationReturned,
  groupSubagentItems,
  parentStreamItems,
} from "./subagent-items";

const item = (id: string, parentID: string, depth: number): ChatTurnItem => ({
  kind: "message",
  id: `message-${id}`,
  role: "assistant",
  content: id,
  createdAt: 0,
  metadata: { "opencode/child-session": { id, parentID, depth } },
});

/** Builds one delegation tool call in the parent stream, as the store keeps the result. */
const delegation = (
  id: string,
  sessionID: string | undefined,
  status: string,
  toolStatus: ChatToolCall["status"] = "completed",
): ChatToolCall => ({
  kind: "toolCall",
  id,
  title: "subagent",
  status: toolStatus,
  content: [],
  locations: [],
  ...(sessionID === undefined
    ? {}
    : { rawOutput: { metadata: { sessionID, status } } }),
  createdAt: 0,
  updatedAt: 0,
});

describe("explicit child projection tree", () => {
  it("keeps concurrent siblings separate instead of nesting by depth", () => {
    expect(
      groupSubagentItems([item("left", "root", 1), item("right", "root", 1)]),
    ).toMatchObject([
      { sourceId: "left", children: [] },
      { sourceId: "right", children: [] },
    ]);
  });
  it("connects a late parent and keeps unrelated root content", () => {
    const root: ChatTurnItem = {
      kind: "message",
      id: "root",
      role: "assistant",
      content: "parent",
      createdAt: 0,
    };
    const groups = groupSubagentItems([
      root,
      item("grandchild", "child", 2),
      item("child", "root", 1),
    ]);
    expect(groups).toHaveLength(2);
    expect(groups[0]).toEqual(root);
    expect(groups[1]).toMatchObject({
      sourceId: "child",
      children: [{ sourceId: "grandchild" }],
    });
  });

  it("keeps cycles, missing parents and invalid sources readable", () => {
    const groups = groupSubagentItems([
      item("a", "b", 2),
      item("b", "a", 1),
      item("orphan", "missing", 2),
    ]);
    expect(groups).toHaveLength(3);
    for (const group of groups)
      expect(group).toMatchObject({ relationshipUnknown: true, children: [] });
    const invalid = item("bad", "root", 0);
    expect(groupSubagentItems([invalid])).toEqual([invalid]);
  });
});

describe("delegation result association", () => {
  it("links a child only through the provider's session id", () => {
    const stream: ChatTurnItem[] = [
      delegation("call", "child", "completed"),
      delegation("other", "sibling", "completed"),
    ];
    expect(delegationReturned(stream, "child")).toBe(true);
    expect(delegationReturned(stream, "sibling")).toBe(true);
    expect(delegationReturned(stream, "missing")).toBe(false);
  });

  it("takes the latest call by persisted position, not the newest timestamp", () => {
    const stream: ChatTurnItem[] = [
      delegation("first", "child", "completed"),
      delegation("second", "child", "running"),
    ];
    expect(delegationReturned(stream, "child")).toBe(false);
    const completedAgain: ChatTurnItem[] = [
      delegation("second", "child", "running"),
      delegation("third", "child", "completed"),
    ];
    expect(delegationReturned(completedAgain, "child")).toBe(true);
  });

  it("requires the provider to report completion, not a settled or failed call", () => {
    expect(
      delegationReturned([delegation("call", "child", "running")], "child"),
    ).toBe(false);
    expect(
      delegationReturned(
        [delegation("call", "child", "completed", "failed")],
        "child",
      ),
    ).toBe(false);
    expect(
      delegationReturned([delegation("call", undefined, "completed")], "child"),
    ).toBe(false);
  });

  it("keeps a top-level child scoped to the parent stream", () => {
    const insideAnotherChild = {
      ...delegation("inner", "sibling", "completed"),
      metadata: {
        "opencode/child-session": { id: "worker", parentID: "root", depth: 1 },
      },
    };
    expect(
      delegationReturned(parentStreamItems([insideAnotherChild]), "sibling"),
    ).toBe(false);
    expect(
      delegationReturned(
        parentStreamItems([delegation("call", "sibling", "completed")]),
        "sibling",
      ),
    ).toBe(true);
  });

  it("associates _subagent/update groups by outer toolCallId and flags conflicting links", () => {
    const parentCall: ChatToolCall = {
      kind: "toolCall",
      id: "agent-call-1",
      title: "Task",
      status: "completed",
      content: [
        { type: "content", content: { type: "text", text: "summary" } },
      ],
      locations: [],
      createdAt: 0,
      updatedAt: 1,
    };
    const subagentMsg: ChatTurnItem = {
      kind: "message",
      id: "sub-msg",
      role: "assistant",
      content: "explored",
      createdAt: 0,
      metadata: {
        "_subagent/update": {
          sessionId: "main-session",
          taskId: "task-42",
          toolCallId: "agent-call-1",
          agentType: "Explore",
        },
      },
    };
    const grouped = groupSubagentItems([parentCall, subagentMsg]);
    expect(grouped).toHaveLength(2);
    expect(grouped[1]).toMatchObject({
      kind: "subagentGroup",
      sourceId: "task-42",
      title: "Explore",
      delegationToolCallId: "agent-call-1",
      relationshipUnknown: false,
    });
    expect(
      delegationReturned(
        parentStreamItems([parentCall, subagentMsg]),
        "task-42",
        "agent-call-1",
      ),
    ).toBe(true);

    // Synthesized turn-end completion without output must not report delegation returned.
    const synthesizedParent: ChatToolCall = { ...parentCall, content: [] };
    expect(
      delegationReturned([synthesizedParent], "task-42", "agent-call-1"),
    ).toBe(false);
  });
});
