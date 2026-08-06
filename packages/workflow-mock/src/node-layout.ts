import { Position, type NodeHandle } from "@xyflow/react";

export const WORKFLOW_NODE_WIDTH = 230;
export const WORKFLOW_NODE_INITIAL_HEIGHT = 98;
export const WORKFLOW_NODE_HANDLE_SIZE = 10;
export const WORKFLOW_NODE_ANCHOR_Y = 61;

/** Provides React Flow with initial handle bounds until the browser measures the custom node. */
export const WORKFLOW_NODE_INITIAL_HANDLES = [
  {
    type: "target",
    position: Position.Left,
    x: -WORKFLOW_NODE_HANDLE_SIZE / 2,
    y: WORKFLOW_NODE_ANCHOR_Y - WORKFLOW_NODE_HANDLE_SIZE / 2,
    width: WORKFLOW_NODE_HANDLE_SIZE,
    height: WORKFLOW_NODE_HANDLE_SIZE,
  },
  {
    type: "source",
    position: Position.Right,
    x: WORKFLOW_NODE_WIDTH - WORKFLOW_NODE_HANDLE_SIZE / 2,
    y: WORKFLOW_NODE_ANCHOR_Y - WORKFLOW_NODE_HANDLE_SIZE / 2,
    width: WORKFLOW_NODE_HANDLE_SIZE,
    height: WORKFLOW_NODE_HANDLE_SIZE,
  },
] satisfies NodeHandle[];
