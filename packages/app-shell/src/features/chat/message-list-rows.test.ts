import { describe, expect, it } from "vitest";
import type { ChatTurn } from "@ora/chat";
import {
  buildMessageListRows,
  MESSAGE_LIST_VIRTUALIZE_MIN_ROWS,
  rowIndexForAnchor,
} from "./message-list-rows";

function completedTurn(id: string, prompt: string): ChatTurn {
  return {
    id,
    userMessage: {
      kind: "message",
      id: `${id}-user`,
      role: "user",
      content: prompt,
      createdAt: 1,
    },
    items: [
      {
        kind: "message",
        id: `${id}-assistant`,
        role: "assistant",
        content: `reply to ${prompt}`,
        createdAt: 2,
      },
    ],
    status: "completed",
    stopReason: null,
    error: null,
    createdAt: 1,
  };
}

describe("buildMessageListRows", () => {
  it("places model-change dividers between the turns they were recorded after", () => {
    const rows = buildMessageListRows(
      [completedTurn("turn-1", "First"), completedTurn("turn-2", "Second")],
      [
        {
          id: "change-1",
          afterTurnCount: 1,
          modelName: "Smart",
          createdAt: 150,
        },
      ],
      false,
    );

    expect(rows.map((row) => row.type)).toEqual([
      "user",
      "display",
      "turnMeta",
      "modelChange",
      "user",
      "display",
      "turnMeta",
      "pad",
    ]);
  });

  it("flattens a live activity phase so each atom can be windowed", () => {
    const turn: ChatTurn = {
      id: "live",
      userMessage: {
        kind: "message",
        id: "live-user",
        role: "user",
        content: "go",
        createdAt: 1,
      },
      items: [
        {
          kind: "toolCall",
          id: "read-1",
          title: "Read a.ts",
          toolKind: "read",
          status: "completed",
          content: [],
          locations: [{ path: "a.ts" }],
          createdAt: 2,
          updatedAt: 2,
        },
        {
          kind: "toolCall",
          id: "read-2",
          title: "Read b.ts",
          toolKind: "read",
          status: "in_progress",
          content: [],
          locations: [{ path: "b.ts" }],
          createdAt: 3,
          updatedAt: 3,
        },
      ],
      status: "streaming",
      stopReason: null,
      error: null,
      createdAt: 1,
    };

    const rows = buildMessageListRows([turn], [], false);
    expect(rows.some((row) => row.type === "activityAtom")).toBe(true);
    expect(rows.some((row) => row.type === "turnMeta")).toBe(false);
  });

  it("maps navigator anchors onto the first matching flattened row", () => {
    const turns = [
      completedTurn("turn-1", "First"),
      completedTurn("turn-2", "Second"),
    ];
    const rows = buildMessageListRows(turns, [], false);
    expect(rowIndexForAnchor(rows, turns, "turn-2:user")).toBeGreaterThan(
      rowIndexForAnchor(rows, turns, "turn-1:response"),
    );
    expect(rows[rowIndexForAnchor(rows, turns, "turn-1:response")]?.type).toBe(
      "display",
    );
  });

  it("keeps the virtualize threshold above a short completed exchange", () => {
    const rows = buildMessageListRows(
      [completedTurn("turn-1", "Hi")],
      [],
      false,
    );
    expect(rows.length).toBeLessThan(MESSAGE_LIST_VIRTUALIZE_MIN_ROWS);
  });
});
