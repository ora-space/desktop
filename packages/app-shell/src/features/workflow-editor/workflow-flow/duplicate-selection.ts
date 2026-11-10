import type { Edge, Node } from "@xyflow/react";
import type { WorkflowNodeData } from "@ora/workflow-mock";
import { WORKFLOW_SNAP_GRID } from "./layout";

export const WORKFLOW_DUPLICATE_STEP = WORKFLOW_SNAP_GRID[0];

export type WorkflowCopyNode = Node<WorkflowNodeData, "workflow">;

/** Nodes and the edges that connect two of them, detached from the live graph. */
export interface WorkflowSelectionCopy {
  nodes: WorkflowCopyNode[];
  edges: Edge[];
}

/** True when the key event should stay with a text control instead of the graph. */
export function isTextEntryTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  if (target.closest("input, textarea, select, [role='textbox']") !== null) {
    return true;
  }
  let current: HTMLElement | null = target;
  while (current !== null) {
    const value = current.getAttribute("contenteditable");
    if (
      current.isContentEditable === true ||
      (value !== null && value.toLowerCase() !== "false")
    ) {
      return true;
    }
    current = current.parentElement;
  }
  return false;
}

/**
 * Copies the selected nodes. A selected frame also includes its subgraph.
 * An edge is included only when both ends are in that set.
 */
export function captureSelection(
  nodes: readonly WorkflowCopyNode[],
  edges: readonly Edge[],
): WorkflowSelectionCopy {
  const selected = nodes.filter((node) => node.selected).map((node) => node.id);
  const included = expandSelection(nodes, selected);
  const copiedNodes = nodes.filter((node) => included.has(node.id));
  const copiedEdges = edges.filter(
    (edge) => included.has(edge.source) && included.has(edge.target),
  );
  return {
    nodes: copiedNodes.map((node) => cloneNode(node, false)),
    edges: copiedEdges.map((edge) => cloneEdge(edge)),
  };
}

/** Builds new ids and shifts nodes that are not positioned inside a copied parent. */
export function materializeCopy(
  copy: WorkflowSelectionCopy,
  occupiedIds: Iterable<string>,
  offset: { x: number; y: number },
  omitStart: boolean,
): WorkflowSelectionCopy {
  let nodes = copy.nodes;
  if (omitStart) {
    const dropped = new Set(
      nodes
        .filter(
          (node) =>
            node.data.kind === "start" &&
            node.parentId === undefined &&
            node.data.containerId === undefined,
        )
        .map((node) => node.id),
    );
    if (dropped.size > 0) {
      const removed = expandSelection(nodes, [...dropped]);
      nodes = nodes.filter((node) => !removed.has(node.id));
    }
  }
  if (nodes.length === 0) {
    return { nodes: [], edges: [] };
  }
  const kept = new Set(nodes.map((node) => node.id));
  const taken = new Set(occupiedIds);
  const ordered = [...nodes].sort(
    (left, right) => depth(left, nodes) - depth(right, nodes),
  );
  const idMap = new Map<string, string>();
  for (const node of ordered) {
    const id = nextId(node, idMap, taken);
    idMap.set(node.id, id);
    taken.add(id);
  }
  const createdNodes = ordered.map((node) => {
    const parentId = node.parentId;
    const parentCopied = parentId !== undefined && idMap.has(parentId);
    const clone = cloneNode(node, true);
    clone.id = idMap.get(node.id)!;
    clone.parentId = parentCopied ? idMap.get(parentId) : parentId;
    clone.position = parentCopied
      ? { ...node.position }
      : { x: node.position.x + offset.x, y: node.position.y + offset.y };
    clone.data = remapValue(clone.data, idMap) as WorkflowNodeData;
    return clone;
  });
  const createdEdges = copy.edges
    .filter((edge) => kept.has(edge.source) && kept.has(edge.target))
    .map((edge) => {
      const clone = cloneEdge(edge);
      clone.id = allocateId("edge", taken);
      taken.add(clone.id);
      clone.source = idMap.get(edge.source)!;
      clone.target = idMap.get(edge.target)!;
      if (clone.data !== undefined) {
        clone.data = remapValue(clone.data, idMap) as Edge["data"];
      }
      return clone;
    });
  return { nodes: createdNodes, edges: createdEdges };
}

function expandSelection(
  nodes: readonly WorkflowCopyNode[],
  roots: readonly string[],
): Set<string> {
  const included = new Set(roots);
  const pending = [...roots];
  while (pending.length > 0) {
    const id = pending.pop()!;
    for (const node of nodes) {
      if (included.has(node.id)) {
        continue;
      }
      if (node.parentId === id || node.data.containerId === id) {
        included.add(node.id);
        pending.push(node.id);
      }
    }
  }
  return included;
}

function depth(
  node: WorkflowCopyNode,
  nodes: readonly WorkflowCopyNode[],
): number {
  const byId = new Map(nodes.map((candidate) => [candidate.id, candidate]));
  let level = 0;
  let current = node;
  const seen = new Set<string>();
  while (!seen.has(current.id)) {
    seen.add(current.id);
    const parentId = current.parentId ?? current.data.containerId;
    if (parentId === undefined) {
      break;
    }
    const parent = byId.get(parentId);
    if (parent === undefined) {
      break;
    }
    current = parent;
    level += 1;
  }
  return level;
}

function nextId(
  node: WorkflowCopyNode,
  idMap: ReadonlyMap<string, string>,
  taken: Set<string>,
): string {
  const parentId = node.parentId ?? node.data.containerId;
  if (parentId !== undefined && node.id.startsWith(`${parentId}-`)) {
    const mappedParent = idMap.get(parentId);
    if (mappedParent !== undefined) {
      const candidate = `${mappedParent}${node.id.slice(parentId.length)}`;
      if (!taken.has(candidate)) {
        return candidate;
      }
    }
  }
  return allocateId(node.data.kind, taken);
}

function allocateId(prefix: string, taken: Set<string>): string {
  let sequence = 1;
  while (taken.has(`${prefix}-${sequence}`)) {
    sequence += 1;
  }
  return `${prefix}-${sequence}`;
}

function cloneNode(
  node: WorkflowCopyNode,
  selected: boolean,
): WorkflowCopyNode {
  const clone = structuredClone(node);
  delete clone.selected;
  delete clone.dragging;
  delete clone.measured;
  return { ...clone, selected };
}

function cloneEdge(edge: Edge): Edge {
  const clone = structuredClone(edge);
  delete clone.selected;
  return clone;
}

function remapValue(
  value: unknown,
  idMap: ReadonlyMap<string, string>,
): unknown {
  if (typeof value === "string") {
    const mapped = idMap.get(value);
    if (mapped !== undefined) {
      return mapped;
    }
    const pairs = [...idMap.entries()].sort(
      (left, right) => right[0].length - left[0].length,
    );
    return pairs.reduce(
      (current, [from, to]) =>
        current
          .replaceAll(`{{#${from}.`, `{{#${to}.`)
          .replaceAll(`{{#${from}#`, `{{#${to}#`),
      value,
    );
  }
  if (Array.isArray(value)) {
    return value.map((item) => remapValue(item, idMap));
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, child]) => [
        key,
        remapValue(child, idMap),
      ]),
    );
  }
  return value;
}
