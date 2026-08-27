import { describe, expect, it } from "vitest";
import type { Edge, Node } from "@xyflow/react";
import { WORKFLOW_NODE_WIDTH, type WorkflowNodeData } from "@ora/workflow-mock";
import {
  nodePositionAt,
  organizeWorkflowNodes,
  snapNodePosition,
} from "./layout";

/** Creates the smallest executable node needed to exercise layout behavior. */
function workflowNode(
  id: string,
  x: number,
  y: number,
): Node<WorkflowNodeData, "workflow"> {
  return {
    id,
    type: "workflow",
    position: { x, y },
    data: { kind: "output", title: id, description: "", instruction: "" },
  };
}

describe("workflow-flow layout", () => {
  it("centers a dropped card around the pointer at handle height", () => {
    expect(nodePositionAt({ x: 400, y: 300 })).toEqual({
      x: 400 - WORKFLOW_NODE_WIDTH / 2,
      y: 239,
    });
  });

  it("aligns new node positions to the canvas grid", () => {
    expect(snapNodePosition({ x: 253, y: 207 })).toEqual({ x: 260, y: 200 });
  });

  it("places dependency layers left-to-right and preserves branch order", () => {
    const nodes = [
      workflowNode("start", 900, 300),
      workflowNode("top", 20, 40),
      workflowNode("bottom", 20, 400),
      workflowNode("output", 0, 0),
    ];
    const edges: Edge[] = [
      { id: "e1", source: "start", target: "top" },
      { id: "e2", source: "start", target: "bottom" },
      { id: "e3", source: "top", target: "output" },
      { id: "e4", source: "bottom", target: "output" },
    ];

    const organized = organizeWorkflowNodes(nodes, edges);
    const positions = Object.fromEntries(
      organized.map((node) => [node.id, node.position]),
    );

    expect(positions.start!.x).toBeLessThan(positions.top!.x);
    expect(positions.top!.x).toBe(positions.bottom!.x);
    expect(positions.top!.y).toBeLessThan(positions.bottom!.y);
    expect(positions.bottom!.x).toBeLessThan(positions.output!.x);
  });
});
