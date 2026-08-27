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
  annotations: WorkflowGraphAnnotation[];
  description?: string;
}

/** Serializable editor note kept outside the executable node list. */
export interface WorkflowGraphAnnotation {
  id: string;
  type: "annotation";
  position: { x: number; y: number };
  width?: number;
  height?: number;
  selected?: boolean;
  data: {
    text: string;
    theme: "yellow" | "blue" | "green" | "pink" | "gray";
  };
}

const DEFAULT_VIEWPORT: WorkflowViewport = { x: 0, y: 0, zoom: 1 };
const WORKFLOW_ANNOTATION_THEMES = new Set([
  "yellow",
  "blue",
  "green",
  "pink",
  "gray",
]);

/**
 * Serializes the editor graph into the JSON envelope stored in a snapshot's graph column.
 *
 * The envelope carries serializable geometry, annotations, and the description; the workflow
 * name and timestamps stay on the Workflow record. Unknown fields ride through parsing unchanged.
 */
export function serializeWorkflowGraph(input: {
  nodes: readonly WorkflowDefinitionNode[];
  edges: readonly WorkflowDefinitionEdge[];
  viewport: WorkflowViewport;
  annotations?: readonly WorkflowGraphAnnotation[];
  description?: string;
}): string {
  return JSON.stringify({
    nodes: input.nodes,
    edges: input.edges,
    viewport: input.viewport,
    annotations: input.annotations ?? [],
    ...(input.description === undefined
      ? {}
      : { description: input.description }),
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
    return {
      nodes: [],
      edges: [],
      viewport: DEFAULT_VIEWPORT,
      annotations: [],
    };
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return {
      nodes: [],
      edges: [],
      viewport: DEFAULT_VIEWPORT,
      annotations: [],
    };
  }
  const record = value as Record<string, unknown>;
  // Spread the raw record first so unknown fields added by future versions survive a
  // resave; only the geometry the editor understands is normalized underneath them.
  const envelope: WorkflowGraphEnvelope = {
    ...record,
    nodes: Array.isArray(record.nodes)
      ? (record.nodes as WorkflowDefinitionNode[]).map(upgradeLegacyNodeKind)
      : [],
    edges: Array.isArray(record.edges)
      ? (record.edges as WorkflowDefinitionEdge[])
      : [],
    viewport: isWorkflowViewport(record.viewport)
      ? record.viewport
      : DEFAULT_VIEWPORT,
    annotations: Array.isArray(record.annotations)
      ? record.annotations.filter(isWorkflowGraphAnnotation)
      : [],
  };
  if (typeof record.description === "string") {
    envelope.description = record.description;
  }
  return envelope;
}

/** Guards editor-note data before custom nodes render persisted content. */
function isWorkflowGraphAnnotation(
  value: unknown,
): value is WorkflowGraphAnnotation {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const annotation = value as Record<string, unknown>;
  const position = annotation.position as Record<string, unknown> | undefined;
  const data = annotation.data as Record<string, unknown> | undefined;
  return (
    typeof annotation.id === "string" &&
    annotation.id.trim() !== "" &&
    annotation.type === "annotation" &&
    typeof position?.x === "number" &&
    Number.isFinite(position.x) &&
    typeof position.y === "number" &&
    Number.isFinite(position.y) &&
    typeof data?.text === "string" &&
    typeof data.theme === "string" &&
    WORKFLOW_ANNOTATION_THEMES.has(data.theme)
  );
}

/**
 * Re-maps legacy node kinds that predate the base-node model. The former
 * "prompt" node was renamed to "model" and then folded into the Agent node,
 * which already carries model configuration, so persisted graphs keep loading
 * unchanged as Agent steps.
 */
function upgradeLegacyNodeKind(
  node: WorkflowDefinitionNode,
): WorkflowDefinitionNode {
  const kind = (node.data as { kind?: unknown }).kind;
  if (kind === "prompt" || kind === "model") {
    return { ...node, data: { ...node.data, kind: "agent" } };
  }
  return node;
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
    typeof record.x === "number" &&
    typeof record.y === "number" &&
    typeof record.zoom === "number"
  );
}
