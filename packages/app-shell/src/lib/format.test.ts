import { describe, expect, it } from "vitest";
import { formatElapsedDuration } from "./format";

describe("formatElapsedDuration", () => {
  it("formats second, minute, and hour durations", () => {
    expect(formatElapsedDuration(32_000)).toBe("32s");
    expect(formatElapsedDuration(312_000)).toBe("5m 12s");
    expect(formatElapsedDuration(7_680_000)).toBe("2h 08m");
  });

  it("omits missing or invalid timing", () => {
    expect(formatElapsedDuration(undefined)).toBeNull();
    expect(formatElapsedDuration(Number.NaN)).toBeNull();
    expect(formatElapsedDuration(-1)).toBeNull();
  });
});
