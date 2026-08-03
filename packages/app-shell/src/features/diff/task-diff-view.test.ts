import { describe, expect, it } from "vitest";
import { parseDiff } from "react-diff-view";
import {
  createCommentAnchor,
} from "./task-diff-view";
import { countChanges, parseTaskDiffPatch } from "./task-diff-data";

const PATCH = [
  "diff --git a/src/main.ts b/src/main.ts",
  "index 3bd1f0e..17c13d8 100644",
  "--- a/src/main.ts",
  "+++ b/src/main.ts",
  "@@ -1,2 +1,2 @@",
  " const stable = true;",
  "-const value = 1;",
  "+const value = 2;",
  "",
].join("\n");

describe("task diff view mapping", () => {
  it("maps an empty backend patch to an empty file list", () => {
    expect(parseTaskDiffPatch(" \r\n")).toEqual([]);
  });

  it("counts additions and deletions from parsed backend patches", () => {
    expect(countChanges(parseTaskDiffPatch(PATCH))).toEqual({ additions: 1, deletions: 1 });
  });

  it("maps a parsed insertion to the backend comment anchor contract", () => {
    const [file] = parseDiff(PATCH);
    const [hunk] = file!.hunks;
    const change = hunk!.changes.find((candidate) => candidate.type === "insert")!;

    expect(createCommentAnchor(file!, hunk!, change, "new", "diff-1")).toEqual({
      diffId: "diff-1",
      path: "src/main.ts",
      side: "new",
      startLine: 2,
      endLine: 2,
      hunkHeader: "@@ -1,2 +1,2 @@",
      lineContent: "const value = 2;",
    });
  });

  it("removes the retained carriage return before sending a CRLF anchor", () => {
    const [file] = parseDiff(PATCH.replaceAll("\n", "\r\n"));
    const [hunk] = file!.hunks;
    const change = hunk!.changes.find((candidate) => candidate.type === "insert")!;

    expect(createCommentAnchor(file!, hunk!, change, "new", "diff-2").lineContent)
      .toBe("const value = 2;");
  });
});
