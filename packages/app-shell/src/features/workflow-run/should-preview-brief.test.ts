import { describe, expect, it } from "vitest";
import { shouldPreviewBrief } from "./should-preview-brief";

describe("shouldPreviewBrief", () => {
  it("skips empty and short single-line text", () => {
    expect(shouldPreviewBrief("")).toBe(false);
    expect(shouldPreviewBrief("   ")).toBe(false);
    expect(shouldPreviewBrief("短说明")).toBe(false);
  });

  it("flags long or multi-line text for preview", () => {
    expect(shouldPreviewBrief(`${"很长的说明内容".repeat(20)}`)).toBe(true);
    expect(shouldPreviewBrief("第一行\n第二行")).toBe(true);
  });
});
