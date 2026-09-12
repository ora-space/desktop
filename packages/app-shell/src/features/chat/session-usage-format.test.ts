import { describe, expect, it } from "vitest";
import { formatCompactTokens } from "./session-usage-format";

describe("formatCompactTokens", () => {
  it("uses locale-independent uppercase K M B suffixes", () => {
    expect(formatCompactTokens(999, "zh-CN")).toBe("999");
    expect(formatCompactTokens(1_000, "zh-CN")).toBe("1K");
    expect(formatCompactTokens(14_500, "zh-CN")).toBe("14.5K");
    expect(formatCompactTokens(1_500_000, "zh-CN")).toBe("1.5M");
    expect(formatCompactTokens(2_000_000_000, "zh-CN")).toBe("2B");
  });
});
