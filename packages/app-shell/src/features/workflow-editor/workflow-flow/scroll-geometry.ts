export interface ScreenBox {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** Screen-pixel pan that puts the target box in the middle of the pane. */
export function viewportDeltaToCenter(
  pane: ScreenBox,
  target: ScreenBox,
): { x: number; y: number } {
  return {
    x: pane.left + pane.width / 2 - (target.left + target.width / 2),
    y: pane.top + pane.height / 2 - (target.top + target.height / 2),
  };
}

export interface SelectionAncestorNode {
  id: string;
  parentId?: string;
  data: { kind: string; collapsed?: boolean };
}

/**
 * Iterations that must be expanded before a selected card can be measured.
 * Collapsing a frame unmounts its members, so the scroll button otherwise
 * has nothing on screen to center.
 */
export function collapsedIterationAncestorIds(
  ids: readonly string[],
  nodes: readonly SelectionAncestorNode[],
): string[] {
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const collapsed: string[] = [];
  for (const id of ids) {
    let parentId = byId.get(id)?.parentId;
    while (parentId !== undefined) {
      const parent = byId.get(parentId);
      if (parent === undefined) {
        break;
      }
      if (
        parent.data.kind === "iteration" &&
        parent.data.collapsed === true &&
        !collapsed.includes(parent.id)
      ) {
        collapsed.push(parent.id);
      }
      parentId = parent.parentId;
    }
  }
  return collapsed;
}

/** Bounding box around every selected card, in screen pixels. */
export function unionScreenBox(boxes: readonly ScreenBox[]): ScreenBox | null {
  if (boxes.length === 0) {
    return null;
  }
  const left = Math.min(...boxes.map((box) => box.left));
  const top = Math.min(...boxes.map((box) => box.top));
  const right = Math.max(...boxes.map((box) => box.left + box.width));
  const bottom = Math.max(...boxes.map((box) => box.top + box.height));
  return { left, top, width: right - left, height: bottom - top };
}
