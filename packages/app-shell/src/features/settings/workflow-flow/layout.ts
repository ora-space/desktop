import type { SnapGrid, XYPosition } from "@xyflow/react";
import {
  WORKFLOW_NODE_ANCHOR_Y,
  WORKFLOW_NODE_WIDTH,
} from "@ora/workflow-mock";

export const WORKFLOW_FLOW_NODE_TYPE = "workflow" as const;
export const WORKFLOW_FLOW_EDGE_TYPE = "workflow" as const;
export const WORKFLOW_SNAP_GRID: SnapGrid = [20, 20];

/** Centers a newly placed card around a flow-space point at handle height. */
export function nodePositionAt(point: XYPosition): XYPosition {
  return {
    x: point.x - WORKFLOW_NODE_WIDTH / 2,
    y: point.y - WORKFLOW_NODE_ANCHOR_Y,
  };
}

/** Aligns a top-left node position to the grid rendered by React Flow. */
export function snapNodePosition(position: XYPosition): XYPosition {
  return {
    x: Math.round(position.x / WORKFLOW_SNAP_GRID[0]) * WORKFLOW_SNAP_GRID[0],
    y: Math.round(position.y / WORKFLOW_SNAP_GRID[1]) * WORKFLOW_SNAP_GRID[1],
  };
}
