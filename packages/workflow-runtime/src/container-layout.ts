/** Minimal workflow node shape needed to restore visual container ownership. */
interface WorkflowContainerNode {
  id: string;
  parentId?: string;
  data: {
    containerId?: string;
    kind?: string;
  };
}

/**
 * Canonicalizes Loop parentage from domain-owned `containerId` and orders parents
 * before descendants, as required by React Flow's nested-node layout.
 */
export function workflowContainerNodes<T extends WorkflowContainerNode>(
  nodes: readonly T[],
): T[] {
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const parentById = new Map<string, string>();
  for (const node of nodes) {
    const containerId = node.data.containerId;
    const parentId =
      typeof containerId === "string" && nodeById.has(containerId)
        ? containerId
        : node.parentId;
    if (typeof parentId === "string" && parentId !== node.id) {
      const parent = nodeById.get(parentId);
      if (
        parent !== undefined &&
        (parent.data.kind === undefined ||
          parent.data.kind === "loop" ||
          parent.data.kind === "iteration")
      ) {
        parentById.set(node.id, parentId);
      }
    }
  }
  // Invalid ownership remains in data for execution validation. Only visual parent links
  // are detached: React Flow must never recurse through missing parents or parent cycles.
  const checked = new Set<string>();
  for (const node of nodes) {
    const path = new Set<string>();
    let id: string | undefined = node.id;
    while (id !== undefined && !checked.has(id)) {
      if (path.has(id)) {
        parentById.delete(id);
        break;
      }
      path.add(id);
      id = parentById.get(id);
    }
    for (const member of path) checked.add(member);
  }
  const normalized = nodes.map((node) => {
    const parentId = parentById.get(node.id);
    if (parentId === node.parentId) return node;
    const normalizedNode = { ...node };
    if (parentId === undefined) delete normalizedNode.parentId;
    else normalizedNode.parentId = parentId;
    return normalizedNode;
  });
  const firstIndexById = new Map<string, number>();
  for (const [index, node] of normalized.entries()) {
    if (!firstIndexById.has(node.id)) {
      firstIndexById.set(node.id, index);
    }
  }

  const ordered: T[] = [];
  const visited = new Set<number>();
  for (let index = 0; index < normalized.length; index += 1) {
    const path: number[] = [];
    let current: number | undefined = index;
    while (current !== undefined && !visited.has(current)) {
      visited.add(current);
      path.push(current);
      const parentId: string | undefined = normalized[current]!.parentId;
      current =
        parentId === undefined ? undefined : firstIndexById.get(parentId);
    }
    for (const ancestor of path.reverse()) ordered.push(normalized[ancestor]!);
  }
  return ordered;
}
