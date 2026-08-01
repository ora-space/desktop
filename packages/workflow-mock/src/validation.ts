import { isEdge, isNode } from "@xyflow/react";
import { WORKFLOW_NODE_KINDS } from "./node-data";
import type { DemoWorkflow } from "./fixtures";

/** Validates imported React Flow elements before they enter the canvas. */
export function isDemoWorkflow(value: unknown): value is DemoWorkflow {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Partial<DemoWorkflow>;
  if (!(typeof candidate.id === "string"
    && candidate.id.trim() !== ""
    && typeof candidate.name === "string"
    && candidate.name.trim() !== ""
    && typeof candidate.description === "string"
    && typeof candidate.updatedAt === "string"
    && typeof candidate.viewport === "object"
    && candidate.viewport !== null
    && Number.isFinite(candidate.viewport.x)
    && Number.isFinite(candidate.viewport.y)
    && Number.isFinite(candidate.viewport.zoom)
    && candidate.viewport.zoom > 0
    && Array.isArray(candidate.nodes)
    && Array.isArray(candidate.edges)
    && candidate.nodes.every((node) =>
      isNode(node)
      && typeof node.id === "string"
      && node.id.trim() !== ""
      && node.type === "workflow"
      && Number.isFinite(node.position?.x)
      && Number.isFinite(node.position?.y)
      && typeof node.data === "object"
      && node.data !== null
      && WORKFLOW_NODE_KINDS.includes(node.data.kind)
      && (node.data.kind !== "start" || node.deletable === false)
      && typeof node.data.title === "string"
      && typeof node.data.description === "string"
      && typeof node.data.instruction === "string"
    )
    && candidate.edges.every((edge) =>
      isEdge(edge)
      && typeof edge.id === "string"
      && edge.id.trim() !== ""
      && typeof edge.source === "string"
      && typeof edge.target === "string"
    ))) {
    return false;
  }

  const nodeIds = new Set(candidate.nodes.map((node) => node.id));
  const edgeIds = new Set(candidate.edges.map((edge) => edge.id));
  return nodeIds.size === candidate.nodes.length
    && edgeIds.size === candidate.edges.length
    && new Set([...nodeIds, ...edgeIds]).size === candidate.nodes.length + candidate.edges.length
    && candidate.nodes.filter((node) => node.data.kind === "start").length === 1
    && candidate.edges.every((edge) =>
      edge.source !== edge.target
      && nodeIds.has(edge.source)
      && nodeIds.has(edge.target)
    )
    && new Set(candidate.edges.map((edge) => `${edge.source}\u0000${edge.target}`)).size
      === candidate.edges.length;
}
