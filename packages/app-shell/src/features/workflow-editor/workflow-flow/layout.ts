import type { Edge, Node, SnapGrid, XYPosition } from "@xyflow/react";
import {
  WORKFLOW_NODE_ANCHOR_Y,
  WORKFLOW_NODE_INITIAL_HEIGHT,
  WORKFLOW_NODE_WIDTH,
  type WorkflowNodeData,
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

const WORKFLOW_LAYOUT_COLUMN_GAP = 120;
const WORKFLOW_LAYOUT_ROW_GAP = 80;

/** Arranges executable DAG nodes left-to-right while leaving editor notes untouched. */
export function organizeWorkflowNodes(
  nodes: readonly Node<WorkflowNodeData, "workflow">[],
  edges: readonly Edge[],
): Node<WorkflowNodeData, "workflow">[] {
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const outgoing = new Map(nodes.map((node) => [node.id, [] as string[]]));
  const indegree = new Map(nodes.map((node) => [node.id, 0]));
  for (const edge of edges) {
    if (!nodeById.has(edge.source) || !nodeById.has(edge.target)) {
      continue;
    }
    outgoing.get(edge.source)?.push(edge.target);
    indegree.set(edge.target, (indegree.get(edge.target) ?? 0) + 1);
  }

  const rank = new Map(nodes.map((node) => [node.id, 0]));
  const compareNodes = (leftId: string, rightId: string): number => {
    const left = nodeById.get(leftId)!;
    const right = nodeById.get(rightId)!;
    return left.position.y - right.position.y || leftId.localeCompare(rightId);
  };
  const queue = nodes
    .filter((node) => indegree.get(node.id) === 0)
    .map((node) => node.id)
    .sort(compareNodes);
  const visited = new Set<string>();
  while (queue.length > 0) {
    const source = queue.shift()!;
    visited.add(source);
    for (const target of (outgoing.get(source) ?? []).sort(compareNodes)) {
      rank.set(
        target,
        Math.max(rank.get(target) ?? 0, (rank.get(source) ?? 0) + 1),
      );
      const nextIndegree = (indegree.get(target) ?? 1) - 1;
      indegree.set(target, nextIndegree);
      if (nextIndegree === 0) {
        queue.push(target);
        queue.sort(compareNodes);
      }
    }
  }

  // Invalid cyclic imports still receive a deterministic final column instead of blocking layout.
  const finalRank = Math.max(0, ...rank.values()) + 1;
  for (const node of nodes) {
    if (!visited.has(node.id)) {
      rank.set(node.id, finalRank);
    }
  }

  const columns = new Map<number, Node<WorkflowNodeData, "workflow">[]>();
  for (const node of nodes) {
    const column = rank.get(node.id) ?? 0;
    columns.set(column, [...(columns.get(column) ?? []), node]);
  }

  const positions = new Map<string, XYPosition>();
  for (const [column, columnNodes] of [...columns.entries()].sort(
    ([left], [right]) => left - right,
  )) {
    columnNodes.sort((left, right) => compareNodes(left.id, right.id));
    const heights = columnNodes.map(
      (node) =>
        node.measured?.height ?? node.height ?? WORKFLOW_NODE_INITIAL_HEIGHT,
    );
    const totalHeight =
      heights.reduce((total, height) => total + height, 0) +
      Math.max(0, columnNodes.length - 1) * WORKFLOW_LAYOUT_ROW_GAP;
    let y = -totalHeight / 2;
    for (const [index, node] of columnNodes.entries()) {
      positions.set(
        node.id,
        snapNodePosition({
          x: column * (WORKFLOW_NODE_WIDTH + WORKFLOW_LAYOUT_COLUMN_GAP),
          y,
        }),
      );
      y += heights[index]! + WORKFLOW_LAYOUT_ROW_GAP;
    }
  }

  return nodes.map((node) => ({ ...node, position: positions.get(node.id)! }));
}
