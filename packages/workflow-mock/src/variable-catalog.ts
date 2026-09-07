import type { Edge, Node } from "@xyflow/react";
import type {
  WorkflowGlobalVariable,
  WorkflowNodeData,
  WorkflowVariableValueType,
} from "./node-data";

/** Built-in globals available in every workflow. Runtime fills their values at run creation. */
export const DEFAULT_WORKFLOW_GLOBAL_VARIABLES: WorkflowGlobalVariable[] = [
  { name: "sys.workflow_id", valueType: "string" },
  { name: "sys.timestamp", valueType: "number" },
];

/** Restores required system globals while preserving user-defined declarations. */
export function normalizeWorkflowGlobalVariables(
  variables: readonly WorkflowGlobalVariable[] | undefined,
): WorkflowGlobalVariable[] {
  const byName = new Map(
    (variables ?? []).map((variable) => [variable.name, variable]),
  );
  for (const required of DEFAULT_WORKFLOW_GLOBAL_VARIABLES) {
    byName.set(required.name, required);
  }
  return [...byName.values()];
}

/** One variable that can be selected by a downstream workflow node. */
export interface WorkflowVariableCatalogEntry {
  selector: string[];
  sourceNodeId: string;
  /** Presentation-only source metadata added by workflow editors that know node titles. */
  sourceNodeTitle?: string;
  /** Distinguishes workflow-wide declarations from values produced by a node. */
  scope?: "global" | "node";
  variableName: string;
  valueType: WorkflowVariableValueType;
}

/** Derives variables from every ancestor while keeping Conditions value-transparent. */
export function deriveWorkflowVariableCatalog(
  nodes: Array<Node<WorkflowNodeData, "workflow">>,
  edges: Edge[],
  consumerNodeId?: string,
  globalVariables: WorkflowGlobalVariable[] = DEFAULT_WORKFLOW_GLOBAL_VARIABLES,
): WorkflowVariableCatalogEntry[] {
  const visibleProducerIds =
    consumerNodeId === undefined
      ? new Set(nodes.map((node) => node.id))
      : collectVisibleProducerIds(nodes, edges, consumerNodeId);
  const entries = globalVariables.flatMap(globalVariableCatalogEntry);

  for (const node of nodes) {
    if (node.data.kind === "start") {
      if (!visibleProducerIds.has(node.id) || node.id === consumerNodeId) {
        continue;
      }
      // `start.input` is the initial prompt value. It is always declared, even before a run
      // supplies text, so downstream configuration can reference the stable selector.
      entries.push(nodeVariable(node, "input", "string"));
      // Additional Start inputs are also outputs of Start, not globals; graph edges bound scope.
      entries.push(
        ...(node.data.inputVariables ?? [])
          .filter((variable) => variable.name.trim() !== "")
          .map((variable) =>
            nodeVariable(node, variable.name.trim(), variable.valueType),
          ),
      );
      continue;
    }
    if (!visibleProducerIds.has(node.id) || node.id === consumerNodeId) {
      continue;
    }

    // Conditions only route control flow; exposing their internal branch decision would let
    // downstream prompts depend on scheduler state as if it were business data.
    if (node.data.kind !== "condition") {
      entries.push(nodeVariable(node, "output", "string"));
    }
    switch (node.data.kind) {
      case "agent": {
        const contract = node.data.agentConfig?.outputContract;
        if (contract?.type === "structured") {
          entries.push(nodeVariable(node, "structured_output", "object"));
          appendStructuredProperties(entries, node, contract.schema, []);
        }
        break;
      }
      case "condition":
        break;
      case "output":
      case "tool":
      case "junction":
      case "human":
      case "loop":
      case "subflow":
        break;
    }
  }
  return entries;
}

/** Collects every upstream ancestor while remaining finite for temporarily cyclic edit graphs. */
function collectVisibleProducerIds(
  nodes: Array<Node<WorkflowNodeData, "workflow">>,
  edges: Edge[],
  consumerNodeId: string,
): Set<string> {
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const incomingByTarget = new Map<string, string[]>();
  for (const edge of edges) {
    const incoming = incomingByTarget.get(edge.target) ?? [];
    incoming.push(edge.source);
    incomingByTarget.set(edge.target, incoming);
  }
  const producers = new Set<string>();
  const visited = new Set<string>();

  /** Adds one ancestor and continues upstream so variables follow the full execution path. */
  const collect = (nodeId: string): void => {
    if (visited.has(nodeId)) {
      return;
    }
    visited.add(nodeId);
    if (!nodeById.has(nodeId)) {
      return;
    }
    producers.add(nodeId);
    for (const upstreamId of incomingByTarget.get(nodeId) ?? []) {
      collect(upstreamId);
    }
  };

  for (const predecessorId of incomingByTarget.get(consumerNodeId) ?? []) {
    collect(predecessorId);
  }
  return producers;
}

/** Adds selectable leaf paths from the supported object-schema subset. */
function appendStructuredProperties(
  entries: WorkflowVariableCatalogEntry[],
  node: Node<WorkflowNodeData, "workflow">,
  schema: Record<string, unknown>,
  path: string[],
): void {
  const properties = schema.properties;
  if (
    properties === null ||
    typeof properties !== "object" ||
    Array.isArray(properties)
  ) {
    return;
  }
  for (const [name, property] of Object.entries(properties)) {
    if (
      property === null ||
      typeof property !== "object" ||
      Array.isArray(property)
    ) {
      continue;
    }
    const propertySchema = property as Record<string, unknown>;
    const nextPath = [...path, name];
    const valueType = schemaValueType(propertySchema);
    entries.push({
      ...nodeVariable(node, "structured_output", valueType),
      selector: [node.id, "structured_output", ...nextPath],
      variableName: `structured_output.${nextPath.join(".")}`,
    });
    if (valueType === "object") {
      appendStructuredProperties(entries, node, propertySchema, nextPath);
    }
  }
}

/** Maps JSON Schema primitive names to the workflow variable type set. */
function schemaValueType(
  schema: Record<string, unknown>,
): WorkflowVariableValueType {
  const value = schema.type;
  if (
    value === "string" ||
    value === "integer" ||
    value === "number" ||
    value === "boolean" ||
    value === "secret" ||
    value === "file" ||
    value === "object" ||
    value === "any"
  ) {
    return value;
  }
  if (value === "array") {
    const items = schema.items;
    if (items === null || typeof items !== "object" || Array.isArray(items)) {
      return "array";
    }
    const itemType = (items as Record<string, unknown>).type;
    if (
      itemType === "string" ||
      itemType === "number" ||
      itemType === "object" ||
      itemType === "boolean" ||
      itemType === "file" ||
      itemType === "any"
    ) {
      return `array[${itemType}]`;
    }
    return "array";
  }
  return "any";
}

/** Converts one qualified global declaration into a selectable catalog entry. */
function globalVariableCatalogEntry(
  variable: WorkflowGlobalVariable,
): WorkflowVariableCatalogEntry[] {
  const parts = variable.name.split(".").filter((part) => part !== "");
  if (parts.length < 2) {
    return [];
  }
  return [
    {
      selector: parts,
      sourceNodeId: parts[0]!,
      variableName: parts.slice(1).join("."),
      valueType: variable.valueType,
    },
  ];
}

/** Builds a catalog entry owned by a workflow node. */
function nodeVariable(
  node: Node<WorkflowNodeData, "workflow">,
  variableName: string,
  valueType: WorkflowVariableValueType,
): WorkflowVariableCatalogEntry {
  return {
    selector: [node.id, variableName],
    sourceNodeId: node.id,
    variableName,
    valueType,
  };
}
