import type { GraphWorkflowNodeStatus, WorkflowDefinition } from "./types";

/** Projected state of one definition node, derived only from persisted facts. */
type ProjectedNodeState =
  | "succeeded"
  | "failed"
  | "cancelled"
  | "running"
  | "inactive"
  | "ready"
  | "unresolved";

/** Resolution of one graph edge, derived from its source node's projected state. */
type EdgeState = "active" | "inactive" | "failed" | "unresolved";

/**
 * Computes which definition nodes are on an inactive branch and will never run, given the
 * committed node statuses and the run's condition decisions.
 *
 * Mirrors the backend branch projection: an edge from a succeeded node is active — a condition
 * edge only when its source handle matches the node's `selected_branch_id`, with an unlabeled
 * edge defaulting to the else branch — and an inactive node propagates inactivity downstream.
 * Condition decisions are keyed by the Condition node id in the public run contract.
 */
export function computeInactiveNodes(
  definition: WorkflowDefinition,
  nodeStatus: Record<string, GraphWorkflowNodeStatus>,
  conditionDecisions: Record<string, string>,
): Set<string> {
  const states = new Map<string, ProjectedNodeState>();
  for (const [nodeId, status] of Object.entries(nodeStatus)) {
    const projection =
      status === "succeeded"
        ? ("succeeded" as const)
        : status === "failed"
          ? ("failed" as const)
          : status === "cancelled"
            ? ("cancelled" as const)
            : status === "running" || status === "awaiting_input"
              ? ("running" as const)
              : null;
    if (projection !== null) {
      states.set(nodeId, projection);
    }
  }

  // Resolve no-run nodes until stable, so branch deactivation propagates downstream.
  let changed = true;
  while (changed) {
    changed = false;
    for (const node of definition.nodes) {
      if (states.has(node.id)) {
        continue;
      }
      const computed = projectNode(
        node.id,
        definition,
        states,
        conditionDecisions,
      );
      if (
        (computed === "ready" || computed === "inactive") &&
        states.get(node.id) !== computed
      ) {
        states.set(node.id, computed);
        changed = true;
      }
    }
  }

  const inactive = new Set<string>();
  for (const [nodeId, state] of states) {
    if (state === "inactive") {
      inactive.add(nodeId);
    }
  }
  return inactive;
}

/** Derives one node's projected state from the resolution of its incoming edges. */
function projectNode(
  nodeId: string,
  definition: WorkflowDefinition,
  states: Map<string, ProjectedNodeState>,
  conditionDecisions: Record<string, string>,
): ProjectedNodeState {
  const incoming = definition.edges.filter((edge) => edge.target === nodeId);
  if (incoming.length === 0) {
    // The start node has no incoming edges and is active.
    return "ready";
  }
  let anyActive = false;
  for (const edge of incoming) {
    const edgeState = resolveEdge(
      edge.source,
      edge.sourceHandle,
      definition,
      states,
      conditionDecisions,
    );
    if (edgeState === "unresolved" || edgeState === "failed") {
      return "unresolved";
    }
    if (edgeState === "active") {
      anyActive = true;
    }
  }
  return anyActive ? "ready" : "inactive";
}

/** Resolves one edge from its source node's projected state and the condition decision. */
function resolveEdge(
  sourceId: string,
  sourceHandle: string | undefined,
  definition: WorkflowDefinition,
  states: Map<string, ProjectedNodeState>,
  conditionDecisions: Record<string, string>,
): EdgeState {
  const sourceState = states.get(sourceId);
  if (sourceState === "succeeded") {
    const sourceNode = definition.nodes.find((node) => node.id === sourceId);
    if (sourceNode?.data.kind === "condition") {
      const selected = conditionDecisions[sourceId];
      // A condition whose decision is unknown (legacy run without a variable pool) leaves its
      // successors unresolved rather than declaring them inactive.
      if (selected === undefined) {
        return "unresolved";
      }
      const active =
        sourceHandle === undefined
          ? selected === "else"
          : sourceHandle === selected;
      return active ? "active" : "inactive";
    }
    return "active";
  }
  if (sourceState === "inactive" || sourceState === "cancelled") {
    return "inactive";
  }
  if (sourceState === "failed") {
    return "failed";
  }
  return "unresolved";
}
