import { describe, expect, it } from "vitest";
import { Position } from "@xyflow/react";
import { workflowConnectionAnchor } from "./path";

describe("workflowConnectionAnchor", () => {
  it("aligns centered preview coordinates with directional edge anchors", () => {
    expect(workflowConnectionAnchor({
      x: 100,
      y: 50,
      position: Position.Right,
      width: 24,
      height: 24,
    })).toEqual({ x: 112, y: 50 });
    expect(workflowConnectionAnchor({
      x: 200,
      y: 80,
      position: Position.Left,
      width: 24,
      height: 24,
    })).toEqual({ x: 188, y: 80 });
  });
});
