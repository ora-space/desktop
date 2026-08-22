import { parseDiff } from "react-diff-view";
import { describe, expect, it } from "vitest";
import {
  canQuoteDiffChange,
  diffQuoteAnchorFor,
  unifiedDiffQuoteLine,
} from "./task-diff-quote";

const PATCH = [
  "diff --git a/src/example.ts b/src/example.ts",
  "index 1111111..2222222 100644",
  "--- a/src/example.ts",
  "+++ b/src/example.ts",
  "@@ -1,3 +1,3 @@",
  " keep",
  "-old line",
  "+new line",
].join("\n");

describe("task diff quote targets", () => {
  it("quotes new-side insert/normal and old-side deletes only", () => {
    const file = parseDiff(PATCH)[0]!;
    const [normal, deleted, inserted] = file.hunks[0]!.changes;

    expect(canQuoteDiffChange(normal!, "new")).toBe(true);
    expect(canQuoteDiffChange(normal!, "old")).toBe(false);
    expect(canQuoteDiffChange(deleted!, "old")).toBe(true);
    expect(canQuoteDiffChange(deleted!, "new")).toBe(false);
    expect(canQuoteDiffChange(inserted!, "new")).toBe(true);
    expect(canQuoteDiffChange(inserted!, "old")).toBe(false);
  });

  it("keeps source text on the anchor and restores unified markers for the agent", () => {
    const file = parseDiff(PATCH)[0]!;
    const [normal, deleted, inserted] = file.hunks[0]!.changes;

    const context = diffQuoteAnchorFor(file, normal!, "new", "n")!;
    const gone = diffQuoteAnchorFor(file, deleted!, "old", "d")!;
    const added = diffQuoteAnchorFor(file, inserted!, "new", "i")!;
    expect(context.content).toBe("keep");
    expect(unifiedDiffQuoteLine(context.changeType, context.content)).toBe(
      " keep",
    );
    expect(unifiedDiffQuoteLine(gone.changeType, gone.content)).toBe(
      "-old line",
    );
    expect(unifiedDiffQuoteLine(added.changeType, added.content)).toBe(
      "+new line",
    );
  });

  it("resolves old-side deletes to the old path", () => {
    const file = parseDiff(PATCH)[0]!;
    const deleted = file.hunks[0]!.changes[1]!;
    const anchor = diffQuoteAnchorFor(file, deleted, "old", "d")!;
    expect(anchor.path).toBe("src/example.ts");
    expect(anchor.side).toBe("old");
    expect(anchor.content).toBe("old line");
  });
});
