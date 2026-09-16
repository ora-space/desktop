import type { Node } from "@xyflow/react";
import {
  WORKFLOW_ITERATION_NODE_HEIGHT,
  WORKFLOW_ITERATION_NODE_WIDTH,
  type WorkflowNodeData,
} from "@ora/workflow-mock";

/** The editable workflow draft shape the containment helper works over. */
export interface ContainmentWorkflow {
  nodes: Node<WorkflowNodeData, "workflow">[];
}

/** Frame geometry of one iteration node on the canvas. */
interface IterationFrame {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

const COLLAPSED_FRAME_HEIGHT = 112;

/** Strips containment fields so a reassignment never carries stale values. */
function withoutContainment(
  node: Node<WorkflowNodeData, "workflow">,
): Omit<Node<WorkflowNodeData, "workflow">, "parentId" | "extent"> {
  const next: Record<string, unknown> = { ...node };
  delete next.parentId;
  delete next.extent;
  return next as Omit<
    Node<WorkflowNodeData, "workflow">,
    "parentId" | "extent"
  >;
}

/**
 * Derives iteration containment from the dragged nodes' final positions.
 *
 * A node whose center lands inside an iteration frame becomes that frame's member
 * (`parentId` + `extent: "parent"`, position converted to frame-relative); a member whose
 * center lands outside its frame returns to the outer canvas (position converted back to
 * absolute). Output and nested iteration nodes never become members — the region boundary
 * rules forbid them — and frames themselves never nest. Returns the same workflow reference
 * when nothing changed so callers can skip a no-op history step.
 */
export function applyIterationContainment(
  workflow: ContainmentWorkflow,
  draggedNodes: Node<WorkflowNodeData, "workflow">[],
): ContainmentWorkflow {
  const frames: IterationFrame[] = workflow.nodes
    .filter((node) => node.data.kind === "iteration")
    .map((node) => ({
      id: node.id,
      x: node.position.x,
      y: node.position.y,
      width: WORKFLOW_ITERATION_NODE_WIDTH,
      height:
        node.data.collapsed === true
          ? COLLAPSED_FRAME_HEIGHT
          : WORKFLOW_ITERATION_NODE_HEIGHT,
    }));
  if (frames.length === 0 && draggedNodes.length === 0) {
    return workflow;
  }
  const frameById = new Map(frames.map((frame) => [frame.id, frame]));
  const iterationIds = new Set(frames.map((frame) => frame.id));
  const absolutePositionOf = (node: Node<WorkflowNodeData, "workflow">) => {
    if (node.parentId === undefined || !iterationIds.has(node.parentId)) {
      return { x: node.position.x, y: node.position.y };
    }
    const frame = frameById.get(node.parentId);
    if (frame === undefined) {
      return node.position;
    }
    return { x: node.position.x + frame.x, y: node.position.y + frame.y };
  };

  let changed = false;
  const nextNodes = workflow.nodes.map((node) => {
    if (!draggedNodes.some((dragged) => dragged.id === node.id)) {
      return node;
    }
    // Frames and forbidden member kinds keep their containment untouched; the structural
    // validation on publish stays the authoritative rejection path.
    if (
      node.data.kind === "iteration" ||
      node.data.kind === "output" ||
      node.data.kind === "start"
    ) {
      return node;
    }
    const absolute = absolutePositionOf(node);
    const center = { x: absolute.x + 115, y: absolute.y + 49 };
    const inside = frames.find(
      (frame) =>
        center.x >= frame.x &&
        center.x <= frame.x + frame.width &&
        center.y >= frame.y &&
        center.y <= frame.y + frame.height,
    );
    const currentParent =
      node.parentId !== undefined && iterationIds.has(node.parentId)
        ? node.parentId
        : null;
    if (inside !== undefined && inside.id !== currentParent) {
      changed = true;
      return {
        ...withoutContainment(node),
        parentId: inside.id,
        extent: "parent" as const,
        position: {
          x: Math.max(0, absolute.x - inside.x),
          y: Math.max(0, absolute.y - inside.y),
        },
      };
    }
    if (inside === undefined && currentParent !== null) {
      changed = true;
      return { ...withoutContainment(node), position: absolute };
    }
    return node;
  });
  return changed ? { ...workflow, nodes: nextNodes } : workflow;
}
