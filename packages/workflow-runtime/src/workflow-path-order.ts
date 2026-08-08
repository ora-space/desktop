import type { WorkflowDefinition, WorkflowDefinitionNode } from "./types";

/**
 * Linearizes a workflow DAG for path UI (Theater rail, parallel act lists).
 *
 * Topological constraints are absolute: a node never precedes a predecessor.
 * Among nodes that are concurrently ready in Kahn's algorithm, order by canvas
 * position (x, then y), then stable id — matching Overview reading order without
 * mutating the frozen definition snapshot array.
 */
export function workflowPathOrder(definition: WorkflowDefinition): string[] {
  const nodes = definition.nodes;
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const ids = nodes.map((node) => node.id);
  const idSet = new Set(ids);
  const indegree = new Map(ids.map((id) => [id, 0]));
  const adjacency = new Map(ids.map((id) => [id, [] as string[]]));

  for (const edge of definition.edges) {
    if (!idSet.has(edge.source) || !idSet.has(edge.target)) {
      continue;
    }
    adjacency.get(edge.source)!.push(edge.target);
    indegree.set(edge.target, (indegree.get(edge.target) ?? 0) + 1);
  }

  /** Position tie-break for the Kahn ready set (and cycle leftovers). */
  function compareReady(left: string, right: string): number {
    const leftNode = byId.get(left);
    const rightNode = byId.get(right);
    const leftX = leftNode?.position.x ?? 0;
    const rightX = rightNode?.position.x ?? 0;
    if (leftX !== rightX) {
      return leftX < rightX ? -1 : 1;
    }
    const leftY = leftNode?.position.y ?? 0;
    const rightY = rightNode?.position.y ?? 0;
    if (leftY !== rightY) {
      return leftY < rightY ? -1 : 1;
    }
    return left.localeCompare(right);
  }

  const ready = ids.filter((id) => (indegree.get(id) ?? 0) === 0);
  ready.sort(compareReady);

  const order: string[] = [];
  while (ready.length > 0) {
    const id = ready.shift()!;
    order.push(id);
    for (const next of adjacency.get(id) ?? []) {
      const nextDegree = (indegree.get(next) ?? 1) - 1;
      indegree.set(next, nextDegree);
      if (nextDegree === 0) {
        ready.push(next);
        ready.sort(compareReady);
      }
    }
  }

  // Cycles are rejected at deploy time; append any leftovers deterministically.
  const leftovers = ids.filter((id) => !order.includes(id));
  leftovers.sort(compareReady);
  order.push(...leftovers);
  return order;
}

/** Same order as {@link workflowPathOrder}, resolved to definition nodes. */
export function workflowPathNodes(
  definition: WorkflowDefinition,
): WorkflowDefinitionNode[] {
  const byId = new Map(definition.nodes.map((node) => [node.id, node]));
  return workflowPathOrder(definition).flatMap((id) => {
    const node = byId.get(id);
    return node === undefined ? [] : [node];
  });
}
