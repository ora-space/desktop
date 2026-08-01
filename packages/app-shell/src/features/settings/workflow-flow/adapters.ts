import {
  MarkerType,
  Position,
  type Edge,
  type Node,
  type SnapGrid,
} from "@xyflow/react";
import type {
  WorkflowEdge,
  WorkflowNode,
  WorkflowNodeConfig,
  WorkflowNodeKind,
} from "@ora/workflow-mock";

export const WORKFLOW_FLOW_NODE_TYPE = "workflow" as const;
export const WORKFLOW_FLOW_EDGE_TYPE = "workflow" as const;

export const NODE_WIDTH = 230;
/** Approximate card height used until React Flow measures the real DOM node. */
export const NODE_HEIGHT = 98;
/** Vertical offset from the card top to the connection handle center. */
export const NODE_ANCHOR_Y = 61;
export const WORKFLOW_SNAP_GRID: SnapGrid = [20, 20];

export interface WorkflowFlowNodeData {
  kind: WorkflowNodeKind;
  title: string;
  description: string;
  config: WorkflowNodeConfig;
  [key: string]: unknown;
}

export type WorkflowFlowNode = Node<WorkflowFlowNodeData, typeof WORKFLOW_FLOW_NODE_TYPE>;
export type WorkflowFlowEdge = Edge;

/** Declares left/right ports so edges can resolve before DOM measurement completes. */
function workflowHandles() {
  return [
    {
      type: "target" as const,
      position: Position.Left,
      x: 0,
      y: NODE_ANCHOR_Y - 12,
      width: 24,
      height: 24,
    },
    {
      type: "source" as const,
      position: Position.Right,
      x: NODE_WIDTH - 24,
      y: NODE_ANCHOR_Y - 12,
      width: 24,
      height: 24,
    },
  ];
}

/** Maps domain nodes into React Flow nodes while preserving selection from the parent. */
export function toFlowNodes(
  nodes: WorkflowNode[],
  selectedNodeId: string | null,
): WorkflowFlowNode[] {
  return nodes.map((node) => ({
    id: node.id,
    type: WORKFLOW_FLOW_NODE_TYPE,
    position: node.position,
    selected: selectedNodeId === node.id,
    width: NODE_WIDTH,
    height: NODE_HEIGHT,
    initialWidth: NODE_WIDTH,
    initialHeight: NODE_HEIGHT,
    handles: workflowHandles(),
    data: {
      kind: node.kind,
      title: node.title,
      description: node.description,
      config: node.config,
    },
    style: { width: NODE_WIDTH },
  }));
}

/** Maps domain edges into React Flow edges with optional local edge selection. */
export function toFlowEdges(
  edges: WorkflowEdge[],
  nodes: WorkflowNode[],
  selectedEdgeId: string | null,
): WorkflowFlowEdge[] {
  const titles = new Map(nodes.map((node) => [node.id, node.title]));
  return edges.map((edge) => ({
    id: edge.id,
    type: WORKFLOW_FLOW_EDGE_TYPE,
    source: edge.source,
    target: edge.target,
    label: edge.label,
    selected: selectedEdgeId === edge.id,
    reconnectable: true,
    markerEnd: {
      type: MarkerType.ArrowClosed,
      width: 28,
      height: 28,
      markerUnits: "userSpaceOnUse",
      color: selectedEdgeId === edge.id
        ? "var(--ring)"
        : "color-mix(in oklch, var(--foreground) 64%, transparent)",
    },
    data: {
      sourceTitle: titles.get(edge.source) ?? edge.source,
      targetTitle: titles.get(edge.target) ?? edge.target,
    },
  }));
}

/** Centers a newly placed card around a flow-space point at the connection handle. */
export function nodePositionAt(point: { x: number; y: number }): { x: number; y: number } {
  return {
    x: point.x - NODE_WIDTH / 2,
    y: point.y - NODE_ANCHOR_Y,
  };
}

/** Aligns a node's top-left position to the same grid rendered by the workflow canvas. */
export function snapNodePosition(position: { x: number; y: number }): { x: number; y: number } {
  return {
    x: Math.round(position.x / WORKFLOW_SNAP_GRID[0]) * WORKFLOW_SNAP_GRID[0],
    y: Math.round(position.y / WORKFLOW_SNAP_GRID[1]) * WORKFLOW_SNAP_GRID[1],
  };
}
