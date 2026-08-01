import { describe, expect, it } from "vitest";
import { Position } from "@xyflow/react";
import type { WorkflowNode } from "@ora/workflow-mock";
import {
  NODE_HEIGHT,
  NODE_WIDTH,
  nodePositionAt,
  snapNodePosition,
  toFlowEdges,
  toFlowNodes,
} from "./adapters";

describe("workflow-flow adapters", () => {
  const nodes: WorkflowNode[] = [
    {
      id: "start",
      kind: "start",
      title: "开始",
      description: "entry",
      position: { x: 10, y: 20 },
      config: { instruction: "go" },
    },
    {
      id: "out",
      kind: "output",
      title: "输出",
      description: "exit",
      position: { x: 200, y: 20 },
      config: { instruction: "done" },
    },
  ];

  it("maps domain nodes into React Flow nodes with selection and card width", () => {
    expect(toFlowNodes(nodes, "out")).toEqual([
      {
        id: "start",
        type: "workflow",
        position: { x: 10, y: 20 },
        selected: false,
        width: NODE_WIDTH,
        height: NODE_HEIGHT,
        initialWidth: NODE_WIDTH,
        initialHeight: NODE_HEIGHT,
        handles: [
          {
            type: "target",
            position: Position.Left,
            x: 0,
            y: 49,
            width: 24,
            height: 24,
          },
          {
            type: "source",
            position: Position.Right,
            x: NODE_WIDTH - 24,
            y: 49,
            width: 24,
            height: 24,
          },
        ],
        data: {
          kind: "start",
          title: "开始",
          description: "entry",
          config: { instruction: "go" },
        },
        style: { width: NODE_WIDTH },
      },
      {
        id: "out",
        type: "workflow",
        position: { x: 200, y: 20 },
        selected: true,
        width: NODE_WIDTH,
        height: NODE_HEIGHT,
        initialWidth: NODE_WIDTH,
        initialHeight: NODE_HEIGHT,
        handles: [
          {
            type: "target",
            position: Position.Left,
            x: 0,
            y: 49,
            width: 24,
            height: 24,
          },
          {
            type: "source",
            position: Position.Right,
            x: NODE_WIDTH - 24,
            y: 49,
            width: 24,
            height: 24,
          },
        ],
        data: {
          kind: "output",
          title: "输出",
          description: "exit",
          config: { instruction: "done" },
        },
        style: { width: NODE_WIDTH },
      },
    ]);
  });

  it("maps domain edges with endpoint titles for accessible labels", () => {
    expect(
      toFlowEdges(
        [{ id: "e1", source: "start", target: "out", label: "ok" }],
        nodes,
        "e1",
      ),
    ).toEqual([
      {
        id: "e1",
        type: "workflow",
        source: "start",
        target: "out",
        label: "ok",
        selected: true,
        reconnectable: true,
        markerEnd: {
          type: "arrowclosed",
          width: 28,
          height: 28,
          markerUnits: "userSpaceOnUse",
          color: "var(--ring)",
        },
        data: {
          sourceTitle: "开始",
          targetTitle: "输出",
        },
      },
    ]);
  });

  it("centers a dropped card around the pointer at the handle height", () => {
    expect(nodePositionAt({ x: 400, y: 300 })).toEqual({
      x: 400 - NODE_WIDTH / 2,
      y: 300 - 61,
    });
  });

  it("aligns new node positions to the canvas grid", () => {
    expect(snapNodePosition({ x: 253, y: 207 })).toEqual({
      x: 260,
      y: 200,
    });
  });
});
