import { describe, it, expect } from "vitest";
import type { ChatToolCall } from "@ora/chat";
import { parseOpenSpecStatus } from "./openspec-status";

function toolCall(overrides: Partial<ChatToolCall>): ChatToolCall {
  return {
    kind: "toolCall",
    id: "t1",
    title: "openspec status",
    content: [],
    locations: [],
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

const STATUS = {
  changeName: "add-dark-mode",
  artifacts: [
    { id: "proposal", status: "done" },
    { id: "specs", status: "pending" },
  ],
  isComplete: false,
};

describe("parseOpenSpecStatus", () => {
  it("reads status from an already-parsed rawOutput object", () => {
    const result = parseOpenSpecStatus([toolCall({ rawOutput: STATUS })]);
    expect(result).toEqual(STATUS);
  });

  it("parses pure JSON stdout from a text content block", () => {
    const result = parseOpenSpecStatus([
      toolCall({ content: [{ type: "content", content: { type: "text", text: JSON.stringify(STATUS) } }] }),
    ]);
    expect(result?.artifacts).toHaveLength(2);
    expect(result?.changeName).toBe("add-dark-mode");
  });

  it("extracts JSON embedded in surrounding log text", () => {
    const wrapped = `Loading change status...\n${JSON.stringify(STATUS)}\nDone`;
    const result = parseOpenSpecStatus([toolCall({ rawOutput: wrapped })]);
    expect(result?.isComplete).toBe(false);
    expect(result?.artifacts[0]).toEqual({ id: "proposal", status: "done" });
  });

  it("returns null for a terminal-only tool call with no inline output", () => {
    const result = parseOpenSpecStatus([
      toolCall({ content: [{ type: "terminal", terminalId: "term-1" }] }),
    ]);
    expect(result).toBeNull();
  });

  it("returns null for unrelated or malformed output", () => {
    expect(parseOpenSpecStatus([toolCall({ rawOutput: "ok, done" })])).toBeNull();
    expect(parseOpenSpecStatus([toolCall({ rawOutput: { foo: "bar" } })])).toBeNull();
    expect(parseOpenSpecStatus([])).toBeNull();
  });

  it("prefers the most recent parseable status", () => {
    const older = { ...STATUS, changeName: "old" };
    const newer = { ...STATUS, changeName: "new" };
    const result = parseOpenSpecStatus([
      toolCall({ id: "a", rawOutput: older }),
      toolCall({ id: "b", rawOutput: newer }),
    ]);
    expect(result?.changeName).toBe("new");
  });
});
