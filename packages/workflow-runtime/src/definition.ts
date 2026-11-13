import type {
  WorkflowDefinition,
  WorkflowGlobalVariable,
  WorkflowNodeData,
  WorkflowPosition,
  WorkflowViewport,
} from "./types";
import { workflowContainerNodes } from "./container-layout";

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
  parentId?: string;
  deletable?: boolean;
  initialWidth?: number;
  initialHeight?: number;
  width?: number;
  height?: number;
}

export interface WorkflowDefinitionInputEdge {
  id: string;
  source: string;
  target: string;
  type?: string;
  label?: unknown;
  data?: Record<string, unknown>;
  sourceHandle?: string | null;
  targetHandle?: string | null;
}

/** Editor-facing shape accepted at the deploy boundary before normalization. */
export interface WorkflowDefinitionInput {
  id: string;
  name: string;
  description: string;
  updatedAt: string;
  viewport: WorkflowViewport;
  globalVariables?: readonly WorkflowGlobalVariable[];
  nodes: readonly WorkflowDefinitionInputNode[];
  edges: readonly WorkflowDefinitionInputEdge[];
}

/** Removes React Flow runtime fields while preserving unfinished authoring topology. */
export function normalizeWorkflowDocument(
  input: WorkflowDefinitionInput,
): WorkflowDefinition {
  const containerNodes = workflowContainerNodes(input.nodes);
  const definition: WorkflowDefinition = {
    id: input.id,
    name: input.name,
    description: input.description,
    updatedAt: input.updatedAt,
    viewport: { ...input.viewport },
    ...(input.globalVariables === undefined
      ? {}
      : { globalVariables: structuredClone([...input.globalVariables]) }),
    nodes: containerNodes.map((node) => {
      const initialWidth = node.width ?? node.initialWidth;
      const initialHeight = node.height ?? node.initialHeight;
      return {
        id: node.id,
        type: "workflow",
        position: { ...node.position },
        data: normalizeWorkflowNodeData(node.data),
        ...(node.parentId === undefined ? {} : { parentId: node.parentId }),
        ...(node.deletable === undefined ? {} : { deletable: node.deletable }),
        ...(initialWidth === undefined ? {} : { initialWidth }),
        ...(initialHeight === undefined ? {} : { initialHeight }),
      };
    }),
    edges: input.edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      ...(edge.type === "workflow" ? { type: "workflow" as const } : {}),
      ...(typeof edge.label === "string" ? { label: edge.label } : {}),
      ...(edge.data === undefined ? {} : { data: structuredClone(edge.data) }),
      ...(typeof edge.sourceHandle === "string"
        ? { sourceHandle: edge.sourceHandle }
        : {}),
      ...(typeof edge.targetHandle === "string"
        ? { targetHandle: edge.targetHandle }
        : {}),
    })),
  };
  validateWorkflowDocument(definition);
  return definition;
}

/** Migrates deprecated Start instruction data while preserving all other node configuration. */
function normalizeWorkflowNodeData(data: WorkflowNodeData): WorkflowNodeData {
  const cloned = structuredClone(data);
  if (
    cloned.kind === "start" &&
    cloned.input === undefined &&
    cloned.instruction !== undefined
  ) {
    const { instruction, ...startData } = cloned;
    return { ...startData, input: instruction };
  }
  // A saved interactive agent defaults to the explicit completion signal. Legacy frozen graphs are
  // read as `end_turn` by the backend, but the editor writes the new policy on the first save so a
  // re-opened workflow cannot keep the old "any EndTurn is complete" reading (ADR D7).
  if (
    cloned.kind === "agent" &&
    cloned.agentConfig?.interactive === true &&
    cloned.agentConfig.completionSignal == null
  ) {
    return {
      ...cloned,
      agentConfig: {
        ...cloned.agentConfig,
        schemaVersion: 4,
        completionSignal: "agent",
      },
    };
  }
  return cloned;
}

/**
 * Checks document identity and geometry without imposing runtime reachability or DAG rules.
 */
function validateWorkflowDocument(definition: WorkflowDefinition): void {
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

/** Preserves the stricter executable contract used by the in-memory runtime. */
export function normalizeWorkflowDefinition(
  input: WorkflowDefinitionInput,
): WorkflowDefinition {
  const definition = normalizeWorkflowDocument(input);
  validateWorkflowDefinition(definition);
  return definition;
}

/** Execution adapters validate DAGs separately from authoring document persistence. */
export function validateWorkflowDefinition(
  definition: WorkflowDefinition,
): void {
  validateWorkflowDocument(definition);
  const adjacency = new Map(
    definition.nodes.map((node) => [node.id, [] as string[]]),
  );
  const indegree = new Map(definition.nodes.map((node) => [node.id, 0]));
  for (const edge of definition.edges) {
    adjacency.get(edge.source)!.push(edge.target);
    indegree.set(edge.target, indegree.get(edge.target)! + 1);
  }
  const queue = definition.nodes
    .filter((node) => indegree.get(node.id) === 0)
    .map((node) => node.id);
  for (let index = 0; index < queue.length; index += 1) {
    for (const target of adjacency.get(queue[index]!) ?? []) {
      const degree = indegree.get(target)! - 1;
      indegree.set(target, degree);
      if (degree === 0) queue.push(target);
    }
  }
  if (queue.length !== definition.nodes.length) {
    throw new WorkflowDefinitionValidationError(["graph must be acyclic"]);
  }
}
