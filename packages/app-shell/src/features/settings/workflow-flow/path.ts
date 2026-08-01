import { Position } from "@xyflow/react";

/** Aligns React Flow's centered preview coordinates with committed edge anchors. */
export function workflowConnectionAnchor({
  x,
  y,
  position,
  width,
  height,
}: {
  x: number;
  y: number;
  position: Position;
  width: number;
  height: number;
}): { x: number; y: number } {
  switch (position) {
    case Position.Left:
      return { x: x - width / 2, y };
    case Position.Right:
      return { x: x + width / 2, y };
    case Position.Top:
      return { x, y: y - height / 2 };
    case Position.Bottom:
      return { x, y: y + height / 2 };
  }
}
