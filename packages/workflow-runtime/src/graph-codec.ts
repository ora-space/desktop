import type {
  WorkflowDefinitionEdge,
  WorkflowDefinitionNode,
  WorkflowViewport,
} from "./types";

/** The persisted graph envelope: editor geometry plus optional metadata. */
export interface WorkflowGraphEnvelope {
  nodes: WorkflowDefinitionNode[];
  edges: WorkflowDefinitionEdge[];
  viewport: WorkflowViewport;
  description?: string;
}

const DEFAULT_VIEWPORT: WorkflowViewport = { x: 0, y: 0, zoom: 1 };

/**
 * Serializes the editor graph into the JSON envelope stored in a snapshot's graph column.
 *
 * The envelope carries only serializable geometry and the description; the workflow name and
 * timestamps stay on the Workflow record, so description is the sole editor metadata entering
 * the graph string. Unknown fields ride through the JSON round-trip unchanged.
 */
export function serializeWorkflowGraph(input: {
  nodes: readonly WorkflowDefinitionNode[];
  edges: readonly WorkflowDefinitionEdge[];
  viewport: WorkflowViewport;
  description?: string;
}): string {
  return JSON.stringify({
    nodes: input.nodes,
    edges: input.edges,
    viewport: input.viewport,
    ...(input.description === undefined ? {} : { description: input.description }),
  });
}

/**
 * Parses a snapshot graph string back into the editor envelope.
 *
 * Tolerates malformed or partial envelopes: invalid JSON or missing arrays collapse to empty,
 * a missing viewport falls back to the origin, and unknown fields survive the JSON round-trip.
 */
export function parseWorkflowGraph(graph: string): WorkflowGraphEnvelope {
  let value: unknown;
  try {
    value = JSON.parse(graph);
  } catch {
    return { nodes: [], edges: [], viewport: DEFAULT_VIEWPORT };
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { nodes: [], edges: [], viewport: DEFAULT_VIEWPORT };
  }
  const record = value as Record<string, unknown>;
  // Spread the raw record first so unknown fields added by future versions survive a
  // resave; only the geometry the editor understands is normalized underneath them.
  const envelope: WorkflowGraphEnvelope = {
    ...record,
    nodes: Array.isArray(record.nodes)
      ? (record.nodes as WorkflowDefinitionNode[])
      : [],
    edges: Array.isArray(record.edges)
      ? (record.edges as WorkflowDefinitionEdge[])
      : [],
    viewport: isWorkflowViewport(record.viewport) ? record.viewport : DEFAULT_VIEWPORT,
  };
  if (typeof record.description === "string") {
    envelope.description = record.description;
  }
  return envelope;
}

/** Converts a backend epoch-millis timestamp into the editor's ISO string form. */
export function workflowTimestampToIso(millis: bigint | number): string {
  return new Date(Number(millis)).toISOString();
}

/** Converts the editor's ISO timestamp into the backend's epoch-millis form. */
export function isoToWorkflowTimestamp(iso: string): bigint {
  return BigInt(Date.parse(iso));
}

/** Guards the persisted viewport shape before the editor trusts its coordinates. */
function isWorkflowViewport(value: unknown): value is WorkflowViewport {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record.x === "number"
    && typeof record.y === "number"
    && typeof record.zoom === "number"
  );
}
