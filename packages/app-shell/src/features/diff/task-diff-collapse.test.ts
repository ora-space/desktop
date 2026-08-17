import { parseDiff } from "react-diff-view";
import { describe, expect, it } from "vitest";
import {
  buildCollapsedDiffSegments,
  findNewSideLineTarget,
} from "./task-diff-collapse";

const COMPLETE_CONTEXT_PATCH = [
  "diff --git a/src/example.ts b/src/example.ts",
  "index 1111111..2222222 100644",
  "--- a/src/example.ts",
  "+++ b/src/example.ts",
  "@@ -1,20 +1,20 @@",
  ...Array.from({ length: 9 }, (_, index) => ` line ${index + 1}`),
  "-const value = 10;",
  "+const value = 20;",
  ...Array.from({ length: 10 }, (_, index) => ` line ${index + 11}`),
].join("\n");

describe("collapsed task diff sections", () => {
  it("keeps three context lines around changes and collapses distant content", () => {
    const file = parseDiff(COMPLETE_CONTEXT_PATCH)[0]!;
    const segments = buildCollapsedDiffSegments(file.hunks, new Set());

    expect(
      segments.map((segment) =>
        segment.kind === "collapsed"
          ? ["collapsed", segment.lineCount]
          : ["hunk", segment.hunk.changes.length],
      ),
    ).toEqual([
      ["collapsed", 6],
      ["hunk", 8],
      ["collapsed", 7],
    ]);
  });

  it("restores an expanded block from the original complete-context hunk", () => {
    const file = parseDiff(COMPLETE_CONTEXT_PATCH)[0]!;
    const collapsed = buildCollapsedDiffSegments(file.hunks, new Set());
    const firstBlock = collapsed.find(
      (segment) => segment.kind === "collapsed",
    );
    expect(firstBlock?.kind).toBe("collapsed");

    const expanded = buildCollapsedDiffSegments(
      file.hunks,
      new Set(firstBlock?.kind === "collapsed" ? [firstBlock.key] : []),
    );

    expect(expanded.map((segment) => segment.kind)).toEqual([
      "hunk",
      "hunk",
      "collapsed",
    ]);
    expect(
      expanded[0]?.kind === "hunk" ? expanded[0].hunk.changes.length : 0,
    ).toBe(6);
  });
});

describe("findNewSideLineTarget", () => {
  it("finds a visible new-side change without a collapsed key", () => {
    const file = parseDiff(COMPLETE_CONTEXT_PATCH)[0]!;
    const target = findNewSideLineTarget(file.hunks, 10);

    expect(target?.collapsedKey).toBeNull();
    expect(target?.change.content).toContain("const value = 20;");
  });

  it("returns the collapsed block that hides a distant new-side line", () => {
    const file = parseDiff(COMPLETE_CONTEXT_PATCH)[0]!;
    const target = findNewSideLineTarget(file.hunks, 1);
    const firstCollapsed = buildCollapsedDiffSegments(
      file.hunks,
      new Set(),
    ).find((segment) => segment.kind === "collapsed");

    expect(target?.collapsedKey).toBe(
      firstCollapsed?.kind === "collapsed" ? firstCollapsed.key : undefined,
    );
  });

  it("returns null when the new-side line is not in the patch", () => {
    const file = parseDiff(COMPLETE_CONTEXT_PATCH)[0]!;
    expect(findNewSideLineTarget(file.hunks, 99)).toBeNull();
  });
});
