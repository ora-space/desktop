import { Position } from "@xyflow/react";

/** Builds the editor's connection curve with enough horizontal run to avoid stiff corners. */
export function workflowEdgePath({
  sourceX,
  sourceY,
  targetX,
  targetY,
}: {
  sourceX: number;
  sourceY: number;
  targetX: number;
  targetY: number;
}): string {
  // A minimum tangent preserves the softer shape of the original canvas even
  // when nodes are close together or the target sits behind the source.
  const tangent = Math.max(64, Math.abs(targetX - sourceX) * 0.45);
  return [
    `M ${sourceX} ${sourceY}`,
    `C ${sourceX + tangent} ${sourceY},`,
    `${targetX - tangent} ${targetY},`,
    `${targetX} ${targetY}`,
  ].join(" ");
}

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
