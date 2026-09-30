import type {
  WorkflowDefinitionEdge,
  WorkflowDefinitionNode,
  WorkflowPosition,
} from "./types";
import { workflowContainerNodes } from "./container-layout";

const RECOVERED_NODE_GAP = 80;
const RECOVERED_CONTAINER_PADDING = 40;
const RECOVERED_CONTAINER_CONTENT_TOP = 140;

/** Guards finite coordinates before React Flow performs arithmetic on imported JSON. */
export function isWorkflowPosition(value: unknown): value is WorkflowPosition {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.x === "number" &&
    Number.isFinite(record.x) &&
    typeof record.y === "number" &&
    Number.isFinite(record.y)
  );
}

/** Fits recovered positions within each visual scope so parent extents cannot collapse children. */
export function workflowNodesGeometry(
  rawNodes: readonly WorkflowDefinitionNode[],
): WorkflowDefinitionNode[] {
  const missingPositionIds = new Set(
    rawNodes
      .filter((node) => !isWorkflowPosition(node.position))
      .map((node) => node.id),
  );
  const nodes = workflowContainerNodes(rawNodes.map(workflowNodeGeometry));
  const recoveredGeometryIds = new Set(missingPositionIds);
  const children = new Map<string | undefined, WorkflowDefinitionNode[]>();
  for (const node of nodes) {
    const members = children.get(node.parentId) ?? [];
    members.push(node);
    children.set(node.parentId, members);
  }
  // Process inner scopes first: an outer sibling must reserve the fitted width of a nested frame.
  const scopes = [...nodes].reverse().filter((node) => children.has(node.id));
  for (const parent of [...scopes, undefined]) {
    const members = children.get(parent?.id) ?? [];
    const left = parent === undefined ? 0 : RECOVERED_CONTAINER_PADDING;
    let x = members.reduce(
      (cursor, node) =>
        missingPositionIds.has(node.id)
          ? cursor
          : Math.max(
              cursor,
              node.position.x + renderNodeSize(node).width + RECOVERED_NODE_GAP,
            ),
      left,
    );
    for (const node of members) {
      if (!missingPositionIds.has(node.id)) continue;
      node.position = {
        x,
        y: parent === undefined ? 0 : RECOVERED_CONTAINER_CONTENT_TOP,
      };
      x += renderNodeSize(node).width + RECOVERED_NODE_GAP;
    }
    if (
      parent === undefined ||
      (parent.data.kind !== "loop" && parent.data.kind !== "iteration")
    )
      continue;
    const recoveredChildren = members.some((node) =>
      recoveredGeometryIds.has(node.id),
    );
    const previousWidth = parent.initialWidth;
    const previousHeight = parent.initialHeight;
    const size = renderNodeSize(parent);
    if (recoveredChildren || parent.initialWidth === undefined) {
      parent.initialWidth = members.reduce(
        (width, node) =>
          Math.max(
            width,
            node.position.x +
              renderNodeSize(node).width +
              RECOVERED_CONTAINER_PADDING,
          ),
        size.width,
      );
    }
    if (recoveredChildren || parent.initialHeight === undefined) {
      parent.initialHeight = members.reduce(
        (height, node) =>
          Math.max(
            height,
            node.position.y +
              renderNodeSize(node).height +
              RECOVERED_CONTAINER_PADDING,
          ),
        size.height,
      );
    }
    // A fitted descendant can outgrow an authored ancestor even when its relative position was valid.
    if (
      parent.initialWidth !== previousWidth ||
      parent.initialHeight !== previousHeight
    ) {
      recoveredGeometryIds.add(parent.id);
    }
  }
  return nodes;
}

/** Reserves conservative card bounds before custom nodes have been measured by React Flow. */
function renderNodeSize(node: WorkflowDefinitionNode): {
  width: number;
  height: number;
} {
  const container = node.data.kind === "loop" || node.data.kind === "iteration";
  return {
    width:
      node.initialWidth ??
      (container
        ? node.data.kind === "loop"
          ? 620
          : 560
        : node.data.kind === "condition"
          ? 320
          : 240),
    height:
      node.initialHeight ??
      (container ? (node.data.kind === "loop" ? 300 : 340) : 200),
  };
}

/** Restores presentation fields without inventing or changing executable node configuration. */
function workflowNodeGeometry(
  node: WorkflowDefinitionNode,
): WorkflowDefinitionNode {
  const normalized = {
    ...node,
    type: "workflow" as const,
    position: isWorkflowPosition(node.position)
      ? node.position
      : { x: 0, y: 0 },
    data: {
      ...node.data,
      title: typeof node.data.title === "string" ? node.data.title : node.id,
      description:
        typeof node.data.description === "string" ? node.data.description : "",
    },
  };
  for (const field of ["initialWidth", "initialHeight"] as const) {
    const value = normalized[field];
    if (
      value !== undefined &&
      (typeof value !== "number" || !Number.isFinite(value) || value <= 0)
    ) {
      delete normalized[field];
    }
  }
  return normalized;
}

/** Filters unusable edge records and supplies stable, collision-free presentation identities. */
export function workflowEdgeGeometry(
  rawEdges: unknown[],
  droppedNodeIds: ReadonlySet<string>,
): WorkflowDefinitionEdge[] {
  const edges = rawEdges.filter((value): value is WorkflowDefinitionEdge => {
    if (typeof value !== "object" || value === null || Array.isArray(value))
      return false;
    const edge = value as Record<string, unknown>;
    return (
      typeof edge.source === "string" &&
      edge.source.trim() !== "" &&
      typeof edge.target === "string" &&
      edge.target.trim() !== "" &&
      !droppedNodeIds.has(edge.source) &&
      !droppedNodeIds.has(edge.target)
    );
  });
  // Reserve valid IDs before generating any; an early missing ID must not steal a later authored one.
  const reservedIds = new Set(
    edges.flatMap((edge) =>
      typeof edge.id === "string" && edge.id.trim() !== "" ? [edge.id] : [],
    ),
  );
  const usedIds = new Set<string>();
  return edges.map((edge, index) => {
    let id = edge.id;
    if (typeof id !== "string" || id.trim() === "" || usedIds.has(id)) {
      const base = `workflow-edge-${index}`;
      id = base;
      let suffix = 1;
      while (reservedIds.has(id) || usedIds.has(id)) id = `${base}-${suffix++}`;
    }
    usedIds.add(id);
    const normalized = { ...edge, id };
    for (const field of ["label", "sourceHandle", "targetHandle"] as const) {
      if (
        normalized[field] !== undefined &&
        typeof normalized[field] !== "string"
      )
        delete normalized[field];
    }
    return normalized;
  });
}
