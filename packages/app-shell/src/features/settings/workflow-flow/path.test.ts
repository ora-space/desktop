import { describe, expect, it } from "vitest";
import { Position } from "@xyflow/react";
import {
  workflowConnectionAnchor,
  workflowEdgePath,
} from "./path";

describe("workflowEdgePath", () => {
  it("keeps a soft minimum tangent when nodes are close", () => {
    expect(workflowEdgePath({
      sourceX: 100,
      sourceY: 50,
      targetX: 140,
      targetY: 90,
    })).toBe("M 100 50 C 164 50, 76 90, 140 90");
  });

  it("scales the tangent for distant nodes", () => {
    expect(workflowEdgePath({
      sourceX: 0,
      sourceY: 10,
      targetX: 400,
      targetY: 20,
    })).toBe("M 0 10 C 180 10, 220 20, 400 20");
  });

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
