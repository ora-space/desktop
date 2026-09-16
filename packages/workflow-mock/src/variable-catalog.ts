import type { Edge, Node } from "@xyflow/react";
import type {
  WorkflowGlobalVariable,
  WorkflowIterationConfig,
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

/** Strips the element type from an array variable type; untyped arrays yield `any`. */
function arrayElementType(valueType: string): string {
  if (valueType === "array" || valueType === "array[any]") {
    return "any";
  }
  const match = /^array\[(.+)\]$/.exec(valueType);
  return match === null ? "any" : match[1]!;
}

/** Derives variables from every ancestor while keeping Conditions value-transparent.
 *
 * Iteration regions follow the ADR's scope rules: region members see the iteration's
 * `item`/`index` round bindings plus their in-region upstream products, but never the
 * iteration's own exposed results; outer consumers see the three exposed variables
 * (`output`, `entries`, `failed_count`) whose types stay fixed across error strategies.
 */
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

  // Iteration membership: `parentId` containment, as the frozen graph persists it.
  const iterationIds = new Set(
    nodes
      .filter((node) => node.data.kind === "iteration")
      .map((node) => node.id),
  );
  const owningIterationOf = (nodeId: string): string | null => {
    const parent = nodes.find((node) => node.id === nodeId)?.parentId;
    return parent !== undefined && iterationIds.has(parent) ? parent : null;
  };

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
    if (node.data.kind === "iteration") {
      appendIterationVariables(
        entries,
        node,
        nodes,
        consumerNodeId,
        visibleProducerIds,
      );
      continue;
    }
    if (!visibleProducerIds.has(node.id) || node.id === consumerNodeId) {
      continue;
    }
    // A region member's products stay region-visible only: the consumer sees them when it
    // shares the same region, never from outside.
    const consumerOwner =
      consumerNodeId === undefined ? null : owningIterationOf(consumerNodeId);
    const producerOwner = owningIterationOf(node.id);
    if (
      producerOwner !== null &&
      consumerOwner !== producerOwner &&
      consumerNodeId !== undefined
    ) {
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

/** Adds one iteration node's round bindings and exposed results by consumer scope. */
function appendIterationVariables(
  entries: WorkflowVariableCatalogEntry[],
  node: Node<WorkflowNodeData, "workflow">,
  nodes: Array<Node<WorkflowNodeData, "workflow">>,
  consumerNodeId: string | undefined,
  visibleProducerIds: Set<string>,
): void {
  const memberIds = new Set(
    nodes
      .filter((candidate) => candidate.parentId === node.id)
      .map((candidate) => candidate.id),
  );
  const config = node.data.iterationConfig;
  const consumerIsMember =
    consumerNodeId !== undefined && memberIds.has(consumerNodeId);
  if (consumerIsMember) {
    // Round bindings are region-private: members resolve them per round.
    const iteratorType =
      config === undefined
        ? "any"
        : (nodes
            .flatMap((candidate) =>
              candidate.data.kind === "start"
                ? (candidate.data.inputVariables ?? [])
                : [],
            )
            .find(
              (variable) =>
                config.iteratorSelector.length === 2 &&
                variable.name === config.iteratorSelector[1],
            )?.valueType ?? "array");
    const itemType = arrayElementType(iteratorType);
    entries.push({
      ...nodeVariable(node, "item", itemType as WorkflowVariableValueType),
      variableName: "item",
    });
    entries.push(nodeVariable(node, "index", "number"));
    return;
  }
  if (!visibleProducerIds.has(node.id) || node.id === consumerNodeId) {
    return;
  }
  if (config === undefined) {
    return;
  }
  // Outer consumers see the three exposed variables with fixed types. The element type of
  // `output` follows the collect target's declared type when it resolves.
  const collectType = resolveCollectType(config, nodes);
  entries.push(
    nodeVariable(
      node,
      "output",
      `array[${collectType}]` as WorkflowVariableValueType,
    ),
  );
  entries.push(nodeVariable(node, "entries", "array[object]"));
  entries.push(nodeVariable(node, "failed_count", "number"));
}

/** Resolves the declared type of the collect target's root variable, defaulting to `any`. */
function resolveCollectType(
  config: WorkflowIterationConfig,
  nodes: Array<Node<WorkflowNodeData, "workflow">>,
): string {
  const collectNodeId = config.collectSelector[0];
  const collectVariable = config.collectSelector[1];
  if (collectNodeId === undefined || collectVariable === undefined) {
    return "any";
  }
  const target = nodes.find((node) => node.id === collectNodeId);
  if (target === undefined) {
    return "any";
  }
  if (collectVariable === "structured_output") {
    return "object";
  }
  if (collectVariable === "output") {
    return "string";
  }
  if (target.data.kind === "start") {
    return (
      (target.data.inputVariables ?? []).find(
        (variable) => variable.name === collectVariable,
      )?.valueType ?? "any"
    );
  }
  return "any";
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
