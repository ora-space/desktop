import { Position, type NodeHandle } from "@xyflow/react";

export const WORKFLOW_NODE_WIDTH = 230;
/** Iteration container frame: the card sits on top, the region drop zone below it. */
export const WORKFLOW_ITERATION_NODE_WIDTH = 560;
export const WORKFLOW_ITERATION_NODE_HEIGHT = 340;
export const WORKFLOW_ITERATION_CARD_WIDTH = 300;
/** Y position of the region entry handle inside the container's drop zone. */
export const WORKFLOW_ITERATION_ENTRY_HANDLE_Y = 132;
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
