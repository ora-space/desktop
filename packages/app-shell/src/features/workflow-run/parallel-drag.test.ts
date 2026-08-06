import { describe, expect, it } from "vitest";
import { resolveParallelDragSwitch } from "./parallel-drag";

describe("resolveParallelDragSwitch", () => {
  it("ignores short drags", () => {
    expect(resolveParallelDragSwitch(40, 64, 1, 3)).toBe(null);
  });

  it("moves to the next act when dragging left past the threshold", () => {
    expect(resolveParallelDragSwitch(-80, 64, 0, 3)).toBe(1);
  });

  it("moves to the previous act when dragging right past the threshold", () => {
    expect(resolveParallelDragSwitch(80, 64, 2, 3)).toBe(1);
  });

  it("rubber-bands at the ends without switching", () => {
    expect(resolveParallelDragSwitch(-100, 64, 2, 3)).toBe(null);
    expect(resolveParallelDragSwitch(100, 64, 0, 3)).toBe(null);
  });
});
