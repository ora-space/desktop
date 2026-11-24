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
 *
 * Loops follow the same shape, matching the engine, which assigns only a Loop's named
 * `loopConfig.outputs`: outer consumers see those named outputs; body members see the
 * carried loop variables, their in-body upstream products, and every variable visible to the
 * Loop itself (its upstream, e.g. `start.input`). A body's round Start only marks the body
 * entry, so it contributes no variables. An Aggregator inside a body is the exception: the
 * backend types it against the body's own declarations at parse, so it sees only globals and
 * its in-body upstream products.
 *
 * Loop membership is `data.containerId` (the backend refuses a Loop member without it);
 * Iteration membership is `parentId`.
 */
export function deriveWorkflowVariableCatalog(
  nodes: Array<Node<WorkflowNodeData, "workflow">>,
  edges: Edge[],
  consumerNodeId?: string,
  globalVariables: WorkflowGlobalVariable[] = DEFAULT_WORKFLOW_GLOBAL_VARIABLES,
): WorkflowVariableCatalogEntry[] {
  const kindById = new Map(nodes.map((node) => [node.id, node.data.kind]));
  const owningContainerOf = (nodeId: string): string | null => {
    const member = nodes.find((node) => node.id === nodeId);
    if (member === undefined) {
      return null;
    }
    const loopId = member.data.containerId;
    if (loopId !== undefined) {
      return kindById.get(loopId) === "loop" ? loopId : null;
    }
    const parent = member.parentId;
    return parent !== undefined && kindById.get(parent) === "iteration"
      ? parent
      : null;
  };
  const consumerOwner =
    consumerNodeId === undefined ? null : owningContainerOf(consumerNodeId);
  const consumerIsAggregator =
    consumerNodeId !== undefined &&
    kindById.get(consumerNodeId) === "aggregator";
  // The Loop whose round pool the consumer reads: loop variables and the Loop's upstream.
  const consumerLoopId =
    consumerOwner !== null &&
    kindById.get(consumerOwner) === "loop" &&
    !consumerIsAggregator
      ? consumerOwner
      : null;

  let visibleProducerIds: Set<string>;
  if (consumerNodeId === undefined) {
    visibleProducerIds = new Set(nodes.map((node) => node.id));
  } else {
    visibleProducerIds = collectVisibleProducerIds(
      nodes,
      edges,
      consumerNodeId,
    );
    // A Loop body has no edge from outside: its round pool inherits what the Loop sees.
    if (consumerLoopId !== null) {
      for (const id of collectVisibleProducerIds(
        nodes,
        edges,
        consumerLoopId,
      )) {
        visibleProducerIds.add(id);
      }
    }
  }
  const entries = globalVariables.flatMap(globalVariableCatalogEntry);

  // Aggregator outputs are typed by their candidate selectors (backend parse rule: all
  // candidates share one declared type); chained aggregators converge through a fixed point.
  const aggregatorOutputTypes = deriveAggregatorOutputTypes(
    nodes,
    globalVariables,
  );

  for (const node of nodes) {
    if (node.data.kind === "start") {
      if (!visibleProducerIds.has(node.id) || node.id === consumerNodeId) {
        continue;
      }
      // A Loop's round Start only marks the body entry. The engine never assigns its `input`,
      // so offering it would fail every run. Any Start carrying `containerId` is a round
      // Start, even when the caller passes only the body (the Loop panel does).
      if (node.data.containerId !== undefined) {
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
        globalVariables,
      );
      continue;
    }
    if (node.data.kind === "loop") {
      appendLoopVariables(
        entries,
        node,
        consumerNodeId,
        consumerLoopId,
        visibleProducerIds,
      );
      continue;
    }
    if (!visibleProducerIds.has(node.id) || node.id === consumerNodeId) {
      continue;
    }
    // A container member's products stay container-visible only: the consumer sees them
    // when it shares the same container, never from outside.
    const producerOwner = owningContainerOf(node.id);
    if (
      producerOwner !== null &&
      consumerOwner !== producerOwner &&
      consumerNodeId !== undefined
    ) {
      continue;
    }

    // Conditions only route control flow; exposing their internal branch decision would let
    // downstream prompts depend on scheduler state as if it were business data. Aggregators
    // pass their first assigned candidate through, so their output type is the candidates'
    // common declared type rather than the plain string output of data-producing nodes.
    if (node.data.kind === "aggregator") {
      entries.push(
        nodeVariable(
          node,
          "output",
          (aggregatorOutputTypes.get(node.id) ??
            "any") as WorkflowVariableValueType,
        ),
      );
    } else if (node.data.kind !== "condition") {
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
      case "aggregator":
      case "condition":
        break;
      case "output":
      case "tool":
      case "junction":
      case "human":
      case "subflow":
        break;
    }
  }
  return entries;
}

/**
 * Adds one Loop's variables by consumer scope: carried loop variables for its body members,
 * named outputs for visible outer consumers.
 */
function appendLoopVariables(
  entries: WorkflowVariableCatalogEntry[],
  node: Node<WorkflowNodeData, "workflow">,
  consumerNodeId: string | undefined,
  consumerLoopId: string | null,
  visibleProducerIds: Set<string>,
): void {
  const config = node.data.loopConfig;
  if (consumerLoopId === node.id) {
    for (const variable of config?.variables ?? []) {
      entries.push(nodeVariable(node, variable.name, variable.valueType));
    }
    return;
  }
  if (!visibleProducerIds.has(node.id) || node.id === consumerNodeId) {
    return;
  }
  for (const output of config?.outputs ?? []) {
    if (output.name.trim() !== "") {
      entries.push(
        nodeVariable(node, output.name, loopOutputType(output.name)),
      );
    }
  }
}

/**
 * The type the backend declares for a Loop's named output: `any`, except that a Loop declares
 * `output` as a string like other producing nodes before its named outputs. Matching it keeps
 * aggregator candidate typing in agreement with the parse.
 */
function loopOutputType(name: string): WorkflowVariableValueType {
  return name === "output" ? "string" : "any";
}

/** Adds one iteration node's round bindings and exposed results by consumer scope. */
function appendIterationVariables(
  entries: WorkflowVariableCatalogEntry[],
  node: Node<WorkflowNodeData, "workflow">,
  nodes: Array<Node<WorkflowNodeData, "workflow">>,
  consumerNodeId: string | undefined,
  visibleProducerIds: Set<string>,
  globalVariables: readonly WorkflowGlobalVariable[],
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
        : resolveDeclaredVariableType(
            config.iteratorSelector,
            nodes,
            globalVariables,
          );
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

/** Resolves a selector against its complete owner path so same-named declarations cannot collide. */
function resolveDeclaredVariableType(
  selector: readonly string[],
  nodes: Array<Node<WorkflowNodeData, "workflow">>,
  globalVariables: readonly WorkflowGlobalVariable[],
): string {
  const qualifiedName = selector.join(".");
  const global = globalVariables.find(
    (variable) => variable.name === qualifiedName,
  );
  if (global !== undefined) {
    return global.valueType;
  }
  if (selector.length === 2) {
    const [nodeId, variableName] = selector;
    const owner = nodes.find((node) => node.id === nodeId);
    if (owner?.data.kind === "start") {
      return (
        owner.data.inputVariables?.find(
          (variable) => variable.name === variableName,
        )?.valueType ?? "any"
      );
    }
    if (owner?.data.kind === "iteration") {
      if (variableName === "output") {
        return "array[any]";
      }
      if (variableName === "entries") {
        return "array[object]";
      }
    }
  }
  return "any";
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

/**
 * Derives every aggregator's output type the way the backend declares it: the common type of
 * all candidate selectors, or `any` when the candidates are missing or mixed (the backend then
 * rejects the graph at parse). Iterating to a fixed point lets chained aggregators converge.
 */
function deriveAggregatorOutputTypes(
  nodes: Array<Node<WorkflowNodeData, "workflow">>,
  globalVariables: readonly WorkflowGlobalVariable[],
): Map<string, string> {
  const types = new Map<string, string>();
  for (let pass = 0; pass <= nodes.length; pass += 1) {
    let changed = false;
    for (const node of nodes) {
      if (node.data.kind !== "aggregator") {
        continue;
      }
      const selectors = node.data.aggregatorConfig?.variables ?? [];
      const resolved = selectors.map((selector) =>
        selectorDeclaredType(selector, nodes, globalVariables, types),
      );
      const first = resolved[0];
      const uniform =
        first === undefined
          ? "any"
          : resolved.every((type) => type === first)
            ? first
            : "any";
      if (types.get(node.id) !== uniform) {
        types.set(node.id, uniform);
        changed = true;
      }
    }
    if (!changed) {
      break;
    }
  }
  return types;
}

/** Resolves one selector's declared type, mirroring the backend's variable catalog. */
function selectorDeclaredType(
  selector: readonly string[],
  nodes: Array<Node<WorkflowNodeData, "workflow">>,
  globalVariables: readonly WorkflowGlobalVariable[],
  aggregatorOutputTypes: Map<string, string>,
): string {
  if (selector.length !== 2) {
    return "any";
  }
  const qualifiedName = selector.join(".");
  const global = globalVariables.find(
    (variable) => variable.name === qualifiedName,
  );
  if (global !== undefined) {
    return global.valueType;
  }
  const [nodeId, variableName] = selector;
  const owner = nodes.find((node) => node.id === nodeId);
  if (owner === undefined) {
    return "any";
  }
  if (owner.data.kind === "start") {
    if (variableName === "input") {
      return "string";
    }
    return (
      owner.data.inputVariables?.find(
        (variable) => variable.name === variableName,
      )?.valueType ?? "any"
    );
  }
  if (owner.data.kind === "aggregator") {
    return aggregatorOutputTypes.get(nodeId) ?? "any";
  }
  if (owner.data.kind === "loop") {
    return loopOutputType(variableName ?? "");
  }
  if (owner.data.kind === "iteration") {
    const config = owner.data.iterationConfig;
    if (variableName === "output" && config !== undefined) {
      return `array[${resolveCollectType(config, nodes)}]`;
    }
    if (variableName === "entries") {
      return "array[object]";
    }
    if (variableName === "failed_count") {
      return "number";
    }
    return "any";
  }
  if (variableName === "output") {
    return "string";
  }
  if (variableName === "structured_output") {
    return "object";
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
