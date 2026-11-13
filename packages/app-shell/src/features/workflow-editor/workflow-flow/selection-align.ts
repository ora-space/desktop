import type { Node } from "@xyflow/react";

export type SelectionAlignMode = "left" | "top" | "distribute-x";

/** Counts alignable roots that share one parent. */
export function selectionAlignRootCount(
  nodes: readonly Node[],
  selectedIds: readonly string[],
): number {
  const selectedIdSet = new Set(selectedIds);
  const selected = nodes.filter((node) => selectedIdSet.has(node.id));
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const roots = selected.filter(
    (node) => !hasSelectedAncestor(node, selectedIdSet, byId),
  );
  if (roots.length === 0) {
    return 0;
  }
  const parentId = roots[0]?.parentId;
  if (roots.some((node) => node.parentId !== parentId)) {
    return 0;
  }
  return roots.length;
}

/**
 * Moves selected roots that share one parent. Children of a selected parent
 * are skipped so they are not shifted twice. Returns null when the selection
 * cannot be aligned.
 */
export function alignSelectedNodes<TNode extends Node>(
  nodes: readonly TNode[],
  mode: SelectionAlignMode,
): TNode[] | null {
  const selected = nodes.filter((node) => node.selected === true);
  const selectedIds = new Set(selected.map((node) => node.id));
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const roots = selected.filter(
    (node) => !hasSelectedAncestor(node, selectedIds, byId),
  );
  if (mode === "distribute-x" ? roots.length < 3 : roots.length < 2) {
    return null;
  }
  const parentId = roots[0]?.parentId;
  if (roots.some((node) => node.parentId !== parentId)) {
    return null;
  }

  const nextById = new Map(
    roots.map((node) => [node.id, { ...node, position: { ...node.position } }]),
  );
  if (mode === "left") {
    const target = Math.min(...roots.map((node) => node.position.x));
    for (const node of nextById.values()) {
      node.position.x = target;
    }
  } else if (mode === "top") {
    const target = Math.min(...roots.map((node) => node.position.y));
    for (const node of nextById.values()) {
      node.position.y = target;
    }
  } else {
    const ordered = [...roots].sort(
      (left, right) =>
        left.position.x - right.position.x ||
        left.position.y - right.position.y,
    );
    const first = ordered[0]!;
    const last = ordered[ordered.length - 1]!;
    if (last.position.x === first.position.x) {
      return null;
    }
    const widths = ordered.map(
      (node) => node.measured?.width ?? node.width ?? 0,
    );
    const knownWidths = widths.every((width) => width > 0);
    if (knownWidths) {
      const span = last.position.x - first.position.x;
      const middleWidth = widths
        .slice(0, -1)
        .reduce((sum, width) => sum + width, 0);
      const gap = (span - middleWidth) / (ordered.length - 1);
      let cursor = first.position.x;
      for (let index = 1; index < ordered.length - 1; index += 1) {
        cursor += widths[index - 1]! + gap;
        nextById.get(ordered[index]!.id)!.position.x = cursor;
      }
    } else {
      const step = (last.position.x - first.position.x) / (ordered.length - 1);
      for (let index = 1; index < ordered.length - 1; index += 1) {
        nextById.get(ordered[index]!.id)!.position.x =
          first.position.x + step * index;
      }
    }
  }

  let changed = false;
  const next = nodes.map((node) => {
    const updated = nextById.get(node.id);
    if (updated === undefined) {
      return node;
    }
    if (
      updated.position.x === node.position.x &&
      updated.position.y === node.position.y
    ) {
      return node;
    }
    changed = true;
    return updated;
  });
  return changed ? next : null;
}

function hasSelectedAncestor(
  node: Node,
  selectedIds: ReadonlySet<string>,
  byId: ReadonlyMap<string, Node>,
): boolean {
  let parentId = node.parentId;
  const seen = new Set<string>();
  while (parentId !== undefined && !seen.has(parentId)) {
    if (selectedIds.has(parentId)) {
      return true;
    }
    seen.add(parentId);
    parentId = byId.get(parentId)?.parentId;
  }
  return false;
}
