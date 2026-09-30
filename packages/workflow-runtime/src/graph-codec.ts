import {
  WORKFLOW_NODE_KINDS,
  type WorkflowDefinitionEdge,
  type WorkflowDefinitionNode,
  type WorkflowGlobalVariable,
  type WorkflowViewport,
} from "./types";
import {
  isWorkflowPosition,
  workflowEdgeGeometry,
  workflowNodesGeometry,
} from "./graph-render-geometry";

/** The persisted graph envelope: editor geometry plus optional metadata. */
export interface WorkflowGraphEnvelope {
  schemaVersion?: number;
  nodes: WorkflowDefinitionNode[];
  edges: WorkflowDefinitionEdge[];
  viewport: WorkflowViewport;
  annotations: WorkflowGraphAnnotation[];
  globalVariables: WorkflowGlobalVariable[];
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
const WORKFLOW_NODE_KIND_SET: ReadonlySet<string> = new Set(
  WORKFLOW_NODE_KINDS,
);

/** A parsed graph plus the nodes this version had to drop to render it. */
export interface WorkflowGraphParseResult {
  /** The envelope the editor loads, with unrenderable nodes and their edges removed. */
  envelope: WorkflowGraphEnvelope;
  /** How many nodes were dropped because this version cannot render them. */
  droppedNodeCount: number;
  /** Distinct `data.kind` values among the dropped nodes, in encounter order. */
  droppedNodeKinds: string[];
}

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
  globalVariables?: readonly WorkflowGlobalVariable[];
  description?: string;
}): string {
  const usesLoopContainers = input.nodes.some(
    (node) => node.data.kind === "loop" || node.data.containerId !== undefined,
  );
  return JSON.stringify({
    ...(usesLoopContainers ? { schemaVersion: 2 } : {}),
    nodes: input.nodes,
    edges: input.edges,
    viewport: input.viewport,
    annotations: input.annotations ?? [],
    globalVariables: input.globalVariables ?? [],
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
 *
 * Nodes this version cannot render are already gone from the returned envelope; callers that
 * need to report that loss to the user use {@link parseWorkflowGraphWithReport} instead.
 */
export function parseWorkflowGraph(graph: string): WorkflowGraphEnvelope {
  return parseWorkflowGraphWithReport(graph).envelope;
}

/**
 * Parses a snapshot graph string, reporting the nodes dropped on the way in.
 *
 * Every load path shares this boundary, so the editor, the version preview, and the run
 * projection all see the same sanitized graph. A node is dropped when it is not a usable
 * record or when its `data.kind` falls outside the kinds this version renders; the edges that
 * referenced a dropped node go with it, because an edge into a node that is no longer there
 * cannot render either. Dropping is deliberate: the render catalog is a closed set with no
 * component for an unknown kind, so keeping such a node would take the whole canvas down
 * instead of costing the one node nothing could draw.
 */
export function parseWorkflowGraphWithReport(
  graph: string,
): WorkflowGraphParseResult {
  let value: unknown;
  try {
    value = JSON.parse(graph);
  } catch {
    return {
      envelope: emptyEnvelope(),
      droppedNodeCount: 0,
      droppedNodeKinds: [],
    };
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return {
      envelope: emptyEnvelope(),
      droppedNodeCount: 0,
      droppedNodeKinds: [],
    };
  }
  const record = value as Record<string, unknown>;
  const rawNodes: unknown[] = Array.isArray(record.nodes) ? record.nodes : [];
  const nodes: WorkflowDefinitionNode[] = [];
  const droppedNodeIds = new Set<string>();
  const droppedNodeKinds: string[] = [];
  const renderedNodeIds = new Set<string>();
  for (const raw of rawNodes) {
    // Legacy kinds upgrade before the check so a persisted prompt/model node still lands on
    // its supported replacement rather than being read as an unknown kind and dropped.
    const node = isWorkflowNodeRecord(raw) ? upgradeLegacyNodeKind(raw) : null;
    const kind: unknown = node?.data.kind;
    if (
      node !== null &&
      typeof kind === "string" &&
      WORKFLOW_NODE_KIND_SET.has(kind) &&
      !renderedNodeIds.has(node.id)
    ) {
      renderedNodeIds.add(node.id);
      droppedNodeIds.delete(node.id);
      nodes.push(node);
      continue;
    }
    if (isRecordWithId(raw) && !renderedNodeIds.has(raw.id)) {
      droppedNodeIds.add(raw.id);
    }
    if (typeof kind === "string" && !droppedNodeKinds.includes(kind)) {
      droppedNodeKinds.push(kind);
    }
  }
  // Spread the raw record first so unknown fields added by future versions survive a
  // resave; only the geometry the editor understands is normalized underneath them.
  const envelope: WorkflowGraphEnvelope = {
    ...record,
    nodes: workflowNodesGeometry(nodes),
    edges: workflowEdgeGeometry(
      Array.isArray(record.edges) ? record.edges : [],
      droppedNodeIds,
    ),
    viewport: isWorkflowViewport(record.viewport)
      ? record.viewport
      : DEFAULT_VIEWPORT,
    annotations: Array.isArray(record.annotations)
      ? record.annotations.filter(isWorkflowGraphAnnotation)
      : [],
    globalVariables: Array.isArray(record.globalVariables)
      ? record.globalVariables.filter(isWorkflowGlobalVariable)
      : [],
  };
  if (typeof record.description === "string") {
    envelope.description = record.description;
  } else {
    delete envelope.description;
  }
  return {
    envelope,
    droppedNodeCount: rawNodes.length - nodes.length,
    droppedNodeKinds,
  };
}

/** Builds the envelope an unparseable or malformed graph loads as. */
function emptyEnvelope(): WorkflowGraphEnvelope {
  return {
    nodes: [],
    edges: [],
    viewport: DEFAULT_VIEWPORT,
    annotations: [],
    globalVariables: [],
  };
}

/** Guards workflow-wide variables before exposing persisted data to the editor. */
function isWorkflowGlobalVariable(
  value: unknown,
): value is WorkflowGlobalVariable {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const variable = value as Record<string, unknown>;
  return (
    typeof variable.name === "string" &&
    variable.name.includes(".") &&
    typeof variable.valueType === "string"
  );
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

/** Reads the id off a persisted record so edges into a dropped node can be found again. */
function isRecordWithId(value: unknown): value is { id: string } {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const id = (value as Record<string, unknown>).id;
  return typeof id === "string" && id.trim() !== "";
}

/** Guards a persisted node record before the codec reads its id and `data.kind`. */
function isWorkflowNodeRecord(value: unknown): value is WorkflowDefinitionNode {
  if (!isRecordWithId(value)) {
    return false;
  }
  const data = (value as Record<string, unknown>).data;
  return typeof data === "object" && data !== null && !Array.isArray(data);
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
    isWorkflowPosition(record) &&
    typeof record.zoom === "number" &&
    Number.isFinite(record.zoom) &&
    record.zoom > 0
  );
}
