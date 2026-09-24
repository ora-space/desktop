import type { Edge, Node, NodeChange, XYPosition } from "@xyflow/react";
import {
  WORKFLOW_ITERATION_MEMBER_LEFT,
  WORKFLOW_ITERATION_MEMBER_TOP,
  WORKFLOW_ITERATION_NODE_HEIGHT,
  WORKFLOW_ITERATION_NODE_WIDTH,
  WORKFLOW_NODE_INITIAL_HEIGHT,
  WORKFLOW_NODE_WIDTH,
  type WorkflowNodeData,
} from "@ora/workflow-mock";

export interface IterationGraph {
  nodes: Node<WorkflowNodeData, "workflow">[];
  edges: Edge[];
}

export type IterationInsertion =
  | { type: "entry"; iterationId: string }
  | { type: "edge"; iterationId: string; edgeId: string }
  | {
      type: "output";
      iterationId: string;
      sourceId: string;
      sourceHandle?: string | null;
    };

export interface IterationDeletionRepair<TGraph extends IterationGraph> {
  graph: TGraph;
  clearedCollectSelectorIterationIds: string[];
}

export interface IterationDeletionCascade {
  nodeIds: Set<string>;
  edgeIds: Set<string>;
  memberCount: number;
}

const ITERATION_MEMBER_COLUMN_GAP = 100;
const ITERATION_MEMBER_ROW_GAP = 52;
const ITERATION_FRAME_RIGHT_PADDING = 48;
const ITERATION_FRAME_BOTTOM_PADDING = 40;
const CONDITION_NODE_WIDTH = 320;

/** Hides edges owned by folded regions without changing the persisted graph. */
export function projectIterationEdges(
  graph: IterationGraph,
  collapsedIterationIds: ReadonlySet<string>,
): Edge[] {
  if (collapsedIterationIds.size === 0) {
    return graph.edges;
  }
  const parentByNodeId = new Map(
    graph.nodes.map((node) => [node.id, node.parentId] as const),
  );
  return graph.edges.map((edge) => {
    const sourceParent = parentByNodeId.get(edge.source);
    const targetParent = parentByNodeId.get(edge.target);
    return (sourceParent !== undefined &&
      collapsedIterationIds.has(sourceParent)) ||
      (targetParent !== undefined && collapsedIterationIds.has(targetParent))
      ? { ...edge, hidden: true }
      : edge;
  });
}

/** Inserts one iteration member and rewires the selected insertion point atomically. */
export function insertIterationMember<TGraph extends IterationGraph>(
  graph: TGraph,
  insertion: IterationInsertion,
  inputNode: Node<WorkflowNodeData, "workflow">,
): TGraph {
  const iteration = graph.nodes.find(
    (node) =>
      node.id === insertion.iterationId && node.data.kind === "iteration",
  );
  if (iteration === undefined) {
    return graph;
  }
  const node = prepareIterationMember(
    inputNode,
    insertion.iterationId,
    insertionPosition(graph, insertion, inputNode),
  );
  const occupiedIds = [
    ...graph.nodes.map((candidate) => candidate.id),
    ...graph.edges.map((edge) => edge.id),
    node.id,
  ];
  let edges: Edge[];
  switch (insertion.type) {
    case "entry":
      edges = [
        ...graph.edges,
        {
          id: uniqueGraphId("edge", occupiedIds),
          source: insertion.iterationId,
          sourceHandle: "iteration-entry",
          target: node.id,
          type: "workflow",
        },
      ];
      break;
    case "output":
      edges = [
        ...graph.edges,
        {
          id: uniqueGraphId("edge", occupiedIds),
          source: insertion.sourceId,
          ...(insertion.sourceHandle == null
            ? {}
            : { sourceHandle: insertion.sourceHandle }),
          target: node.id,
          type: "workflow",
        },
      ];
      break;
    case "edge": {
      const edge = graph.edges.find(
        (candidate) => candidate.id === insertion.edgeId,
      );
      if (edge === undefined) {
        return graph;
      }
      const { targetHandle, ...sourceEdge } = edge;
      edges = graph.edges.flatMap((candidate) =>
        candidate.id === edge.id
          ? [
              { ...sourceEdge, target: node.id },
              {
                id: uniqueGraphId("edge", occupiedIds),
                source: node.id,
                target: edge.target,
                type: edge.type,
                ...(targetHandle == null ? {} : { targetHandle }),
              },
            ]
          : [candidate],
      );
      break;
    }
  }
  return resizeIterationFrames(
    { ...graph, nodes: [...graph.nodes, node], edges } as TGraph,
    [insertion.iterationId],
    "expand",
  );
}

/** Expands the selected frames to contain every member without shrinking authored space. */
export function expandIterationFrames<TGraph extends IterationGraph>(
  graph: TGraph,
  iterationIds?: readonly string[],
): TGraph {
  return resizeIterationFrames(graph, iterationIds, "expand");
}

/** Recomputes the smallest valid frame around each selected region. */
export function compactIterationFrames<TGraph extends IterationGraph>(
  graph: TGraph,
  iterationIds?: readonly string[],
): TGraph {
  return resizeIterationFrames(graph, iterationIds, "compact");
}

/** Clears iteration collect selectors whose owning member was deleted, then compacts frames. */
export function repairIterationGraphAfterNodeDeletion<
  TGraph extends IterationGraph,
>(
  graph: TGraph,
  removedNodeIds: ReadonlySet<string>,
): IterationDeletionRepair<TGraph> {
  const clearedCollectSelectorIterationIds: string[] = [];
  const nodes = graph.nodes.map((node) => {
    if (node.data.kind !== "iteration") {
      return node;
    }
    const config = node.data.iterationConfig;
    const collectedNodeId = config?.collectSelector[0];
    if (
      config === undefined ||
      collectedNodeId === undefined ||
      !removedNodeIds.has(collectedNodeId)
    ) {
      return node;
    }
    clearedCollectSelectorIterationIds.push(node.id);
    return {
      ...node,
      data: {
        ...node.data,
        iterationConfig: { ...config, collectSelector: [] },
      },
    };
  });
  return {
    graph: compactIterationFrames({ ...graph, nodes } as TGraph),
    clearedCollectSelectorIterationIds,
  };
}

/** Resolves members and incident edges that must join an iteration-container deletion. */
export function resolveIterationDeletionCascade(
  graph: IterationGraph,
  requestedNodeIds: ReadonlySet<string>,
): IterationDeletionCascade {
  const iterationIds = new Set(
    graph.nodes
      .filter(
        (node) =>
          requestedNodeIds.has(node.id) && node.data.kind === "iteration",
      )
      .map((node) => node.id),
  );
  const nodeIds = new Set(requestedNodeIds);
  let memberCount = 0;
  for (const node of graph.nodes) {
    if (node.parentId !== undefined && iterationIds.has(node.parentId)) {
      nodeIds.add(node.id);
      memberCount += 1;
    }
  }
  // Loop descendants share deletion ownership but do not use the Iteration confirmation UX.
  let added = true;
  while (added) {
    added = false;
    for (const node of graph.nodes) {
      const owner = node.data.containerId;
      if (owner !== undefined && nodeIds.has(owner) && !nodeIds.has(node.id)) {
        nodeIds.add(node.id);
        added = true;
      }
    }
  }
  return {
    nodeIds,
    edgeIds: new Set(
      graph.edges
        .filter((edge) => nodeIds.has(edge.source) || nodeIds.has(edge.target))
        .map((edge) => edge.id),
    ),
    memberCount,
  };
}

/** Returns the persisted expanded size, defaulting old snapshots to the supported minimum. */
export function iterationExpandedSize(
  node: Node<WorkflowNodeData, "workflow">,
): { width: number; height: number } {
  return {
    width: finiteSize(node.initialWidth, WORKFLOW_ITERATION_NODE_WIDTH),
    height: finiteSize(node.initialHeight, WORKFLOW_ITERATION_NODE_HEIGHT),
  };
}

/** Normalizes a new region member without persisting React Flow's presentation-only extent. */
function prepareIterationMember(
  node: Node<WorkflowNodeData, "workflow">,
  iterationId: string,
  position: XYPosition,
): Node<WorkflowNodeData, "workflow"> {
  const serializable = { ...node };
  delete serializable.extent;
  delete serializable.expandParent;
  const data =
    node.data.kind === "agent" && node.data.agentConfig !== undefined
      ? {
          ...node.data,
          agentConfig: { ...node.data.agentConfig, interactive: false },
        }
      : node.data;
  return {
    ...serializable,
    parentId: iterationId,
    position,
    data,
  };
}

/** Places a new member close to the selected graph seam while keeping it inside the frame. */
function insertionPosition(
  graph: IterationGraph,
  insertion: IterationInsertion,
  inputNode: Node<WorkflowNodeData, "workflow">,
): XYPosition {
  const members = graph.nodes.filter(
    (node) => node.parentId === insertion.iterationId,
  );
  // The seam-derived point only approximates free space: midpoints can land on an
  // existing member (an entry seam's virtual source sits exactly on the first member's
  // row), so every placement is finally nudged below whatever it would overlap.
  const clearOfMembers = (position: XYPosition): XYPosition =>
    avoidMemberOverlap(
      members,
      position,
      nodeWidth(inputNode),
      nodeHeight(inputNode),
    );
  if (insertion.type === "entry") {
    const entryTargetIds = new Set(
      graph.edges
        .filter(
          (edge) =>
            edge.source === insertion.iterationId &&
            edge.sourceHandle === "iteration-entry",
        )
        .map((edge) => edge.target),
    );
    const entryMembers = members.filter((member) =>
      entryTargetIds.has(member.id),
    );
    const firstCandidateY =
      entryMembers.length === 0
        ? WORKFLOW_ITERATION_MEMBER_TOP
        : Math.max(
            ...entryMembers.map(
              (member) => member.position.y + nodeHeight(member),
            ),
          ) + ITERATION_MEMBER_ROW_GAP;
    return clearOfMembers({
      x: WORKFLOW_ITERATION_MEMBER_LEFT,
      y: nextFreeEntryRowY(members, inputNode, firstCandidateY),
    });
  }
  if (insertion.type === "edge") {
    const edge = graph.edges.find(
      (candidate) => candidate.id === insertion.edgeId,
    );
    if (edge !== undefined) {
      const source = graph.nodes.find((node) => node.id === edge.source);
      const target = graph.nodes.find((node) => node.id === edge.target);
      if (source !== undefined && target !== undefined) {
        const sourcePosition =
          source.id === insertion.iterationId
            ? { x: 0, y: WORKFLOW_ITERATION_MEMBER_TOP }
            : source.position;
        return clearOfMembers({
          x: Math.max(
            WORKFLOW_ITERATION_MEMBER_LEFT,
            Math.round((sourcePosition.x + target.position.x) / 2),
          ),
          y: Math.max(
            WORKFLOW_ITERATION_MEMBER_TOP,
            Math.round((sourcePosition.y + target.position.y) / 2),
          ),
        });
      }
    }
  }
  if (insertion.type === "output") {
    const source = graph.nodes.find((node) => node.id === insertion.sourceId);
    if (source !== undefined) {
      const siblingTargets = graph.edges
        .filter(
          (edge) =>
            edge.source === insertion.sourceId &&
            edge.sourceHandle === insertion.sourceHandle,
        )
        .map((edge) => graph.nodes.find((node) => node.id === edge.target))
        .filter(
          (target): target is Node<WorkflowNodeData, "workflow"> =>
            target !== undefined,
        );
      return clearOfMembers({
        x: source.position.x + nodeWidth(source) + ITERATION_MEMBER_COLUMN_GAP,
        y: stackedMemberTop(siblingTargets, source.position.y),
      });
    }
  }
  return clearOfMembers({
    x: WORKFLOW_ITERATION_MEMBER_LEFT,
    y: stackedMemberTop(members, WORKFLOW_ITERATION_MEMBER_TOP),
  });
}

/**
 * Moves an insertion point below any member box it would overlap.
 *
 * Seam-derived coordinates only approximate free space — an entry seam's virtual
 * source position, for example, sits on the first member's row, so its midpoint
 * can land exactly on an existing card. Pushing the placement below every
 * overlapped member (and re-checking, since the move itself can hit another
 * member) keeps the new card visible and clickable; the loop terminates because
 * each pass strictly increases the y coordinate.
 */
function avoidMemberOverlap(
  members: Node<WorkflowNodeData, "workflow">[],
  position: XYPosition,
  width: number,
  height: number,
): XYPosition {
  const { x } = position;
  let y = position.y;
  let moved = true;
  while (moved) {
    moved = false;
    for (const member of members) {
      const overlaps =
        x < member.position.x + nodeWidth(member) &&
        x + width > member.position.x &&
        y < member.position.y + nodeHeight(member) &&
        y + height > member.position.y;
      if (overlaps) {
        y = member.position.y + nodeHeight(member) + ITERATION_MEMBER_ROW_GAP;
        moved = true;
      }
    }
  }
  return { x, y };
}

/**
 * Stacks one new member below the real bottom of the anchor members.
 *
 * Card heights vary by kind and content far beyond the fixed placeholder, and a
 * freshly inserted card only gets measured after it renders, so counting
 * anchors with the placeholder height lets a tall rendered card overlap the
 * next placement. Using each anchor's measured-or-estimated bottom keeps
 * consecutive inserts clear of each other; with placeholder-sized anchors the
 * result matches the classic fixed-row stacking.
 */
function stackedMemberTop(
  anchors: Node<WorkflowNodeData, "workflow">[],
  baseTop: number,
): number {
  const lowestBottom = anchors.reduce(
    (bottom, anchor) =>
      Math.max(bottom, anchor.position.y + nodeHeight(anchor)),
    baseTop - ITERATION_MEMBER_ROW_GAP,
  );
  return Math.max(baseTop, lowestBottom + ITERATION_MEMBER_ROW_GAP);
}

/** Finds the next first-column row whose rectangle does not overlap an authored member. */
function nextFreeEntryRowY(
  members: readonly Node<WorkflowNodeData, "workflow">[],
  inputNode: Node<WorkflowNodeData, "workflow">,
  firstCandidateY: number,
): number {
  const candidateRight = WORKFLOW_ITERATION_MEMBER_LEFT + nodeWidth(inputNode);
  const candidateHeight = nodeHeight(inputNode);
  const obstacles = members
    .filter(
      (member) =>
        member.position.x < candidateRight &&
        member.position.x + nodeWidth(member) > WORKFLOW_ITERATION_MEMBER_LEFT,
    )
    .sort((left, right) => left.position.y - right.position.y);
  let candidateY = firstCandidateY;
  for (const obstacle of obstacles) {
    const obstacleBottom = obstacle.position.y + nodeHeight(obstacle);
    const overlapsWithGap =
      candidateY < obstacleBottom + ITERATION_MEMBER_ROW_GAP &&
      candidateY + candidateHeight + ITERATION_MEMBER_ROW_GAP >
        obstacle.position.y;
    if (overlapsWithGap) {
      candidateY = obstacleBottom + ITERATION_MEMBER_ROW_GAP;
    }
  }
  return candidateY;
}

/** Applies either monotonic expansion or compact fitting to selected iteration frames. */
function resizeIterationFrames<TGraph extends IterationGraph>(
  graph: TGraph,
  iterationIds: readonly string[] | undefined,
  mode: "expand" | "compact",
): TGraph {
  const selectedIds = iterationIds === undefined ? null : new Set(iterationIds);
  let nodes = graph.nodes;
  let changed = false;
  // Nested regions read member sizes from the working node list. One map pass can
  // enlarge an inner frame while the outer still sees the pre-expand size, so the
  // next measurement tick expands the ancestor and the canvas appears to thrash.
  // Repeat until a pass is a no-op (bounded by nesting depth).
  for (let pass = 0; pass < nodes.length; pass += 1) {
    let passChanged = false;
    const nextNodes = nodes.map((node) => {
      if (
        node.data.kind !== "iteration" ||
        (selectedIds !== null && !selectedIds.has(node.id))
      ) {
        return node;
      }
      const members = nodes.filter(
        (candidate) => candidate.parentId === node.id,
      );
      const requiredWidth = Math.max(
        WORKFLOW_ITERATION_NODE_WIDTH,
        ...members.map(
          (member) =>
            member.position.x +
            nodeWidth(member) +
            ITERATION_FRAME_RIGHT_PADDING,
        ),
      );
      const requiredHeight = Math.max(
        WORKFLOW_ITERATION_NODE_HEIGHT,
        ...members.map(
          (member) =>
            member.position.y +
            nodeHeight(member) +
            ITERATION_FRAME_BOTTOM_PADDING,
        ),
      );
      const current = iterationExpandedSize(node);
      const width = snapSize(
        mode === "expand"
          ? Math.max(current.width, requiredWidth)
          : requiredWidth,
      );
      const height = snapSize(
        mode === "expand"
          ? Math.max(current.height, requiredHeight)
          : requiredHeight,
      );
      if (node.initialWidth === width && node.initialHeight === height) {
        return node;
      }
      passChanged = true;
      // React Flow pins the wrapper to width/height once a manual resize sets
      // them; mirroring the new size keeps the pinned box from going stale after
      // an automatic compact or expand rewrites the authored frame size.
      const resized = { ...node, initialWidth: width, initialHeight: height };
      if (node.width !== undefined || node.height !== undefined) {
        resized.width = width;
        resized.height = height;
      }
      return resized;
    });
    if (!passChanged) {
      break;
    }
    nodes = nextNodes;
    changed = true;
  }
  return changed ? ({ ...graph, nodes } as TGraph) : graph;
}

/** Mirrors a manual React Flow resize gesture onto iteration frames' authored size. */
export function applyIterationFrameResize(
  nodes: readonly Node<WorkflowNodeData, "workflow">[],
  changes: readonly NodeChange[],
): Node<WorkflowNodeData, "workflow">[] {
  const resizedSizes = new Map<string, { width: number; height: number }>();
  for (const change of changes) {
    // Only gesture frames carry `resizing`; plain measurements must never
    // overwrite the authored frame size.
    if (
      change.type === "dimensions" &&
      change.resizing === true &&
      change.dimensions !== undefined
    ) {
      resizedSizes.set(change.id, change.dimensions);
    }
  }
  if (resizedSizes.size === 0) {
    // Keep the caller's array identity so measurement-only change paths can
    // bail out with `===` instead of allocating a fresh nodes list every tick.
    return nodes as Node<WorkflowNodeData, "workflow">[];
  }
  let changed = false;
  const resized = nodes.map((node) => {
    const size = resizedSizes.get(node.id);
    // Members and annotations resize through node.width; only iteration frames
    // keep their editable size in initialWidth/initialHeight.
    if (size === undefined || node.data.kind !== "iteration") {
      return node;
    }
    const width = snapSize(size.width);
    const height = snapSize(size.height);
    if (node.initialWidth === width && node.initialHeight === height) {
      return node;
    }
    changed = true;
    return { ...node, initialWidth: width, initialHeight: height };
  });
  return changed ? resized : (nodes as Node<WorkflowNodeData, "workflow">[]);
}

/** Returns the visual width used for fitting and insertion. */
function nodeWidth(node: Node<WorkflowNodeData, "workflow">): number {
  // Iteration frames author their box in initialWidth; measured can include
  // subpixel chrome and would keep ratcheting an ancestor frame on every pass.
  if (node.data.kind === "iteration") {
    return finiteSize(
      node.initialWidth ?? node.width ?? node.measured?.width,
      WORKFLOW_ITERATION_NODE_WIDTH,
    );
  }
  return finiteSize(
    node.measured?.width ?? node.width ?? node.initialWidth,
    node.data.kind === "condition" ? CONDITION_NODE_WIDTH : WORKFLOW_NODE_WIDTH,
  );
}

/** Returns the visual height used for fitting and insertion. */
function nodeHeight(node: Node<WorkflowNodeData, "workflow">): number {
  if (node.data.kind === "iteration") {
    return finiteSize(
      node.initialHeight ?? node.height ?? node.measured?.height,
      WORKFLOW_ITERATION_NODE_HEIGHT,
    );
  }
  return finiteSize(
    node.measured?.height ?? node.height ?? node.initialHeight,
    WORKFLOW_NODE_INITIAL_HEIGHT,
  );
}

/** Accepts only positive finite dimensions from persisted or measured geometry. */
function finiteSize(value: number | undefined, fallback: number): number {
  return value !== undefined && Number.isFinite(value) && value > 0
    ? value
    : fallback;
}

/** Aligns frame dimensions with the editor's twenty-pixel grid. */
function snapSize(value: number): number {
  return Math.ceil(value / 20) * 20;
}

/** Produces a stable unused graph id without leaking editor state into the transform. */
function uniqueGraphId(prefix: string, occupiedIds: Iterable<string>): string {
  const occupied = new Set(occupiedIds);
  let sequence = 1;
  while (occupied.has(`${prefix}-${sequence}`)) {
    sequence += 1;
  }
  return `${prefix}-${sequence}`;
}
