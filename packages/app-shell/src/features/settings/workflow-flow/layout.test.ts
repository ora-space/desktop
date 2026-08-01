import { describe, expect, it } from "vitest";
import {
  NODE_WIDTH,
  nodePositionAt,
  snapNodePosition,
} from "./layout";

describe("workflow-flow layout", () => {
  it("centers a dropped card around the pointer at handle height", () => {
    expect(nodePositionAt({ x: 400, y: 300 })).toEqual({
      x: 400 - NODE_WIDTH / 2,
      y: 239,
    });
  });

  it("aligns new node positions to the canvas grid", () => {
    expect(snapNodePosition({ x: 253, y: 207 })).toEqual({ x: 260, y: 200 });
  });
});
