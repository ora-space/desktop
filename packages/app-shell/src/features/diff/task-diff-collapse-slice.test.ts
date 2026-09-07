import { describe, expect, it } from "vitest";
import { parseDiff, type HunkData } from "react-diff-view";
import { sliceHunkData } from "./task-diff-collapse";

function bigModifyPatch(): string {
  const lines: string[] = [];
  for (let i = 0; i < 80; i += 1) {
    lines.push(`-old ${i}`);
    lines.push(`+new ${i}`);
  }
  return [
    "diff --git a/src/big.ts b/src/big.ts",
    "index 1111111..2222222 100644",
    "--- a/src/big.ts",
    "+++ b/src/big.ts",
    `@@ -1,${160} +1,${160} @@`,
    ...lines,
    "",
  ].join("\n");
}

describe("sliceHunkData", () => {
  it("slices a hunk with paired delete/insert into contiguous ranges", () => {
    const file = parseDiff(bigModifyPatch())[0]!;
    const hunk = file.hunks[0]!;
    const first = sliceHunkData(hunk, 0, 40);
    const second = sliceHunkData(hunk, 40, 80);

    // First slice starts where the hunk starts.
    expect(first.oldStart).toBe(hunk.oldStart);
    expect(first.newStart).toBe(hunk.newStart);
    // Second slice starts right after the lines covered by the first slice.
    expect(second.oldStart).toBe(first.oldStart + first.oldLines);
    expect(second.newStart).toBe(first.newStart + first.newLines);
    // Each slice carries exactly the requested slice of change rows.
    expect(first.changes.length).toBe(40);
  });

  it("slices a huge hunk into many line-number-continuous native hunks", () => {
    const file = parseDiff(bigModifyPatch())[0]!;
    const hunk = file.hunks[0]!;
    const size = 40;
    const slices: HunkData[] = [];
    for (let start = 0; start < hunk.changes.length; start += size) {
      slices.push(
        sliceHunkData(hunk, start, Math.min(start + size, hunk.changes.length)),
      );
    }
    expect(slices.length).toBeGreaterThan(1);
    let prevOldEnd = 0;
    let prevNewEnd = 0;
    for (const slice of slices) {
      expect(slice.oldStart).toBe(prevOldEnd + 1);
      expect(slice.newStart).toBe(prevNewEnd + 1);
      prevOldEnd = slice.oldStart + slice.oldLines - 1;
      prevNewEnd = slice.newStart + slice.newLines - 1;
    }
  });
});
