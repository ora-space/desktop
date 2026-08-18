import type {
  WorkflowDefinition,
  WorkflowNodeData,
  WorkflowPosition,
  WorkflowViewport,
} from "./types";

/** Stable validation failure that adapters can map to their transport error model. */
export class WorkflowDefinitionValidationError extends Error {
  readonly issues: readonly string[];

  constructor(issues: readonly string[]) {
    super(`Invalid workflow definition: ${issues.join("; ")}`);
    this.name = "WorkflowDefinitionValidationError";
    this.issues = issues;
  }
}

export interface WorkflowDefinitionInputNode {
  id: string;
  type?: string;
  position: WorkflowPosition;
  data: WorkflowNodeData;
  deletable?: boolean;
  initialWidth?: number;
  initialHeight?: number;
}

export interface WorkflowDefinitionInputEdge {
  id: string;
  source: string;
  target: string;
  type?: string;
  label?: unknown;
  data?: Record<string, unknown>;
}

/** Editor-facing shape accepted at the deploy boundary before normalization. */
export interface WorkflowDefinitionInput {
  id: string;
  name: string;
  description: string;
  updatedAt: string;
  viewport: WorkflowViewport;
  nodes: readonly WorkflowDefinitionInputNode[];
  edges: readonly WorkflowDefinitionInputEdge[];
}

/** Removes React Flow runtime fields and produces the serializable execution DTO. */
export function normalizeWorkflowDefinition(
  input: WorkflowDefinitionInput,
): WorkflowDefinition {
  const definition: WorkflowDefinition = {
    id: input.id,
    name: input.name,
    description: input.description,
    updatedAt: input.updatedAt,
    viewport: { ...input.viewport },
    nodes: input.nodes.map((node) => ({
      id: node.id,
      type: "workflow",
      position: { ...node.position },
      data: structuredClone(node.data),
      ...(node.deletable === undefined ? {} : { deletable: node.deletable }),
      ...(node.initialWidth === undefined
        ? {}
        : { initialWidth: node.initialWidth }),
      ...(node.initialHeight === undefined
        ? {}
        : { initialHeight: node.initialHeight }),
    })),
    edges: input.edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      ...(edge.type === "workflow" ? { type: "workflow" as const } : {}),
      ...(typeof edge.label === "string" ? { label: edge.label } : {}),
      ...(edge.data === undefined ? {} : { data: structuredClone(edge.data) }),
    })),
  };
  validateWorkflowDefinition(definition);
  return definition;
}

/**
 * Rejects graph shapes that the DAG scheduler cannot execute deterministically.
 * Keeping this validation transport-neutral lets memory and Tauri
 * adapters enforce the same client-side deployment contract.
 */
export function validateWorkflowDefinition(
  definition: WorkflowDefinition,
): void {
  const issues: string[] = [];
  if (definition.id.trim() === "") {
    issues.push("definition id must not be empty");
  }
  if (definition.nodes.length === 0) {
    issues.push("at least one node is required");
  }

  const nodeIds = new Set<string>();
  for (const node of definition.nodes) {
    if (node.id.trim() === "") {
      issues.push("node id must not be empty");
    } else if (nodeIds.has(node.id)) {
      issues.push(`duplicate node id ${node.id}`);
    }
    nodeIds.add(node.id);
    if (
      !Number.isFinite(node.position.x) ||
      !Number.isFinite(node.position.y)
    ) {
      issues.push(`node ${node.id || "<empty>"} has a non-finite position`);
    }
  }

  const edgeIds = new Set<string>();
  const adjacency = new Map(
    [...nodeIds].map((nodeId) => [nodeId, [] as string[]]),
  );
  const indegree = new Map([...nodeIds].map((nodeId) => [nodeId, 0]));
  for (const edge of definition.edges) {
    if (edge.id.trim() === "") {
      issues.push("edge id must not be empty");
    } else if (edgeIds.has(edge.id)) {
      issues.push(`duplicate edge id ${edge.id}`);
    }
    edgeIds.add(edge.id);
    if (!nodeIds.has(edge.source) || !nodeIds.has(edge.target)) {
      issues.push(`edge ${edge.id || "<empty>"} references an unknown node`);
      continue;
    }
    adjacency.get(edge.source)!.push(edge.target);
    indegree.set(edge.target, indegree.get(edge.target)! + 1);
  }

  // Kahn's algorithm establishes the scheduler invariant without recursion.
  const queue = [...nodeIds].filter((nodeId) => indegree.get(nodeId) === 0);
  let visited = 0;
  for (let index = 0; index < queue.length; index += 1) {
    const nodeId = queue[index]!;
    visited += 1;
    for (const target of adjacency.get(nodeId) ?? []) {
      const nextDegree = indegree.get(target)! - 1;
      indegree.set(target, nextDegree);
      if (nextDegree === 0) {
        queue.push(target);
      }
    }
  }
  if (visited !== nodeIds.size) {
    issues.push("graph must be acyclic");
  }

  if (
    !Number.isFinite(definition.viewport.x) ||
    !Number.isFinite(definition.viewport.y) ||
    !Number.isFinite(definition.viewport.zoom) ||
    definition.viewport.zoom <= 0
  ) {
    issues.push("viewport must contain finite coordinates and a positive zoom");
  }

  if (issues.length > 0) {
    throw new WorkflowDefinitionValidationError(issues);
  }
}
