import type {
  Edge,
  Node,
  NodeChange,
  SnapGrid,
  XYPosition,
} from "@xyflow/react";
import {
  WORKFLOW_ITERATION_MEMBER_LEFT,
  WORKFLOW_ITERATION_MEMBER_TOP,
  WORKFLOW_NODE_ANCHOR_Y,
  WORKFLOW_NODE_INITIAL_HEIGHT,
  WORKFLOW_NODE_WIDTH,
  type WorkflowNodeData,
} from "@ora/workflow-mock";
import { workflowContainerNodes } from "@ora/workflow-runtime";
import {
  compactIterationFrames,
  iterationExpandedSize,
} from "../workflow-iteration-graph";

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

/** Ignores measurement noise while persisting user-driven moves and resizes. */
export function shouldPersistWorkflowNodeChanges(
  changes: readonly NodeChange[],
): boolean {
  return changes.some(
    (change) =>
      change.type !== "select" &&
      (change.type !== "dimensions" ||
        change.setAttributes !== undefined ||
        change.resizing === true ||
        change.resizing === false),
  );
}

/** True when every change is a plain React Flow size probe (no resize gesture). */
export function isPlainMeasurementOnly(
  changes: readonly NodeChange[],
): boolean {
  return (
    changes.length > 0 &&
    changes.every(
      (change) => change.type === "dimensions" && change.resizing !== true,
    )
  );
}

/**
 * Drops extent-clamp position writes that are not part of a user drag.
 *
 * Iteration/loop children use `extent: "parent"`. An undersized frame clamps
 * members and emits bare position changes (no `dragging` flag). Committing
 * those clamps rewrites React state every frame and thrash the canvas.
 * Real drags always set `dragging` true while moving and false on drop.
 */
export function withoutExtentClampPositions<TNode extends Node = Node>(
  changes: readonly NodeChange<TNode>[],
): NodeChange<TNode>[] {
  return changes.filter(
    (change) =>
      change.type !== "position" ||
      change.dragging === true ||
      change.dragging === false,
  );
}

/**
 * True while React Flow is mid-gesture on any node.
 *
 * Growing iteration frames during that window fights `extent: "parent"`: the
 * clamp bounds move under the pointer and the member appears to jitter,
 * especially near the frame edge. Drop handlers already refit frames once.
 */
export function isNodeDragGestureActive<TNode extends Node = Node>(
  changes: readonly NodeChange<TNode>[],
): boolean {
  return changes.some(
    (change) => change.type === "position" && change.dragging === true,
  );
}

/** True when changes are only selection and/or plain size probes (no authored edit). */
export function isNonAuthoringNodeChanges<TNode extends Node = Node>(
  changes: readonly NodeChange<TNode>[],
): boolean {
  return (
    changes.length > 0 &&
    changes.every(
      (change) =>
        change.type === "select" ||
        (change.type === "dimensions" && change.resizing !== true),
    )
  );
}

/** Compares authored iteration frame boxes so measurement churn can bail out. */
export function iterationFrameSizesEqual(
  left: readonly Node<WorkflowNodeData, "workflow">[],
  right: readonly Node<WorkflowNodeData, "workflow">[],
): boolean {
  const rightById = new Map(
    right
      .filter((node) => node.data.kind === "iteration")
      .map((node) => [node.id, iterationExpandedSize(node)] as const),
  );
  let leftCount = 0;
  for (const node of left) {
    if (node.data.kind !== "iteration") {
      continue;
    }
    leftCount += 1;
    const other = rightById.get(node.id);
    if (other === undefined) {
      return false;
    }
    const size = iterationExpandedSize(node);
    if (size.width !== other.width || size.height !== other.height) {
      return false;
    }
  }
  return leftCount === rightById.size;
}

/** Compares authored node geometry/data, ignoring React Flow live probes. */
export function authoredWorkflowNodesEqual(
  left: readonly Node<WorkflowNodeData, "workflow">[],
  right: readonly Node<WorkflowNodeData, "workflow">[],
): boolean {
  if (left.length !== right.length) {
    return false;
  }
  const rightById = new Map(right.map((node) => [node.id, node]));
  for (const node of left) {
    const other = rightById.get(node.id);
    if (other === undefined) {
      return false;
    }
    if (
      node.parentId !== other.parentId ||
      node.position.x !== other.position.x ||
      node.position.y !== other.position.y ||
      node.initialWidth !== other.initialWidth ||
      node.initialHeight !== other.initialHeight ||
      JSON.stringify(node.data) !== JSON.stringify(other.data)
    ) {
      return false;
    }
  }
  return true;
}

/**
 * Reuses the last projected Loop-child object while the authored node identity
 * is unchanged so sibling drag frames do not invalidate memoized node views.
 */
const containedLoopChildCache = new WeakMap<
  Node<WorkflowNodeData, "workflow">,
  Node<WorkflowNodeData, "workflow">
>();

/** Projects Loop children into bounded, auto-expanding React Flow containers. */
export function containWorkflowCanvasNodes(
  nodes: readonly Node<WorkflowNodeData, "workflow">[],
): Node<WorkflowNodeData, "workflow">[] {
  return workflowContainerNodes(nodes).map((node) => {
    if (node.data.containerId === undefined) {
      return node;
    }
    const cached = containedLoopChildCache.get(node);
    if (cached !== undefined) {
      return cached;
    }
    const projected: Node<WorkflowNodeData, "workflow"> = {
      ...node,
      extent: "parent",
      expandParent: true,
    };
    containedLoopChildCache.set(node, projected);
    return projected;
  });
}

const WORKFLOW_LAYOUT_COLUMN_GAP = 120;
const WORKFLOW_LAYOUT_ROW_GAP = 80;
const CONDITION_NODE_WIDTH = 320;

/** Arranges each iteration DAG first, then the outer DAG using fitted container dimensions. */
export function organizeWorkflowNodes(
  nodes: readonly Node<WorkflowNodeData, "workflow">[],
  edges: readonly Edge[],
): Node<WorkflowNodeData, "workflow">[] {
  const iterationIds = new Set(
    nodes
      .filter((node) => node.data.kind === "iteration")
      .map((node) => node.id),
  );
  let arranged = [...nodes];
  for (const iterationId of iterationIds) {
    const members = arranged.filter((node) => node.parentId === iterationId);
    const memberIds = new Set(members.map((node) => node.id));
    const internalEdges = edges.filter(
      (edge) => memberIds.has(edge.source) && memberIds.has(edge.target),
    );
    const positions = layoutDag(members, internalEdges, {
      x: WORKFLOW_ITERATION_MEMBER_LEFT,
      y: WORKFLOW_ITERATION_MEMBER_TOP,
      centerRows: false,
    });
    arranged = arranged.map((node) =>
      positions.has(node.id)
        ? { ...node, position: positions.get(node.id)! }
        : node,
    );
  }

  arranged = compactIterationFrames({
    nodes: arranged,
    edges: [...edges],
  }).nodes;
  const outerNodes = arranged.filter(
    (node) =>
      node.data.containerId === undefined &&
      (node.parentId === undefined || !iterationIds.has(node.parentId)),
  );
  const outerIds = new Set(outerNodes.map((node) => node.id));
  const outerEdges = edges.filter(
    (edge) => outerIds.has(edge.source) && outerIds.has(edge.target),
  );
  const outerPositions = layoutDag(outerNodes, outerEdges, {
    x: 0,
    y: 0,
    centerRows: true,
  });
  return arranged.map((node) =>
    outerPositions.has(node.id)
      ? { ...node, position: outerPositions.get(node.id)! }
      : node,
  );
}

interface LayoutOrigin extends XYPosition {
  centerRows: boolean;
}

/** Computes deterministic positions for one isolated DAG scope. */
function layoutDag(
  nodes: readonly Node<WorkflowNodeData, "workflow">[],
  edges: readonly Edge[],
  origin: LayoutOrigin,
): Map<string, XYPosition> {
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const outgoing = new Map(nodes.map((node) => [node.id, [] as string[]]));
  const indegree = new Map(nodes.map((node) => [node.id, 0]));
  const layoutNodes = nodes;
  for (const edge of edges) {
    if (!nodeById.has(edge.source) || !nodeById.has(edge.target)) {
      continue;
    }
    outgoing.get(edge.source)?.push(edge.target);
    indegree.set(edge.target, (indegree.get(edge.target) ?? 0) + 1);
  }

  const rank = new Map(layoutNodes.map((node) => [node.id, 0]));
  const compareNodes = (leftId: string, rightId: string): number => {
    const left = nodeById.get(leftId)!;
    const right = nodeById.get(rightId)!;
    return left.position.y - right.position.y || leftId.localeCompare(rightId);
  };
  const queue = layoutNodes
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

  const finalRank = Math.max(0, ...rank.values()) + 1;
  for (const node of layoutNodes) {
    if (!visited.has(node.id)) {
      rank.set(node.id, finalRank);
    }
  }
  const columns = new Map<number, Node<WorkflowNodeData, "workflow">[]>();
  for (const node of layoutNodes) {
    const column = rank.get(node.id) ?? 0;
    columns.set(column, [...(columns.get(column) ?? []), node]);
  }
  const orderedColumns = [...columns.entries()].sort(
    ([left], [right]) => left - right,
  );
  const totalHeights = new Map<number, number>();
  for (const [column, columnNodes] of orderedColumns) {
    totalHeights.set(
      column,
      columnNodes.reduce((total, node) => total + nodeHeight(node), 0) +
        Math.max(0, columnNodes.length - 1) * WORKFLOW_LAYOUT_ROW_GAP,
    );
  }
  const maximumColumnHeight = Math.max(0, ...totalHeights.values());
  const positions = new Map<string, XYPosition>();
  let x = origin.x;
  for (const [column, columnNodes] of orderedColumns) {
    columnNodes.sort((left, right) => compareNodes(left.id, right.id));
    const totalHeight = totalHeights.get(column) ?? 0;
    let y = origin.centerRows
      ? origin.y - totalHeight / 2
      : origin.y + (maximumColumnHeight - totalHeight) / 2;
    let columnWidth = 0;
    for (const node of columnNodes) {
      positions.set(node.id, snapNodePosition({ x, y }));
      y += nodeHeight(node) + WORKFLOW_LAYOUT_ROW_GAP;
      columnWidth = Math.max(columnWidth, nodeWidth(node));
    }
    x += columnWidth + WORKFLOW_LAYOUT_COLUMN_GAP;
  }
  return positions;
}

/** Returns the current expanded width so outer layout reserves the full region. */
function nodeWidth(node: Node<WorkflowNodeData, "workflow">): number {
  if (node.data.kind === "iteration") {
    return iterationExpandedSize(node).width;
  }
  return (
    node.measured?.width ??
    node.width ??
    node.initialWidth ??
    (node.data.kind === "condition"
      ? CONDITION_NODE_WIDTH
      : WORKFLOW_NODE_WIDTH)
  );
}

/** Returns the current expanded height so rows cannot overlap iteration contents. */
function nodeHeight(node: Node<WorkflowNodeData, "workflow">): number {
  if (node.data.kind === "iteration") {
    return iterationExpandedSize(node).height;
  }
  return (
    node.measured?.height ??
    node.height ??
    node.initialHeight ??
    WORKFLOW_NODE_INITIAL_HEIGHT
  );
}
