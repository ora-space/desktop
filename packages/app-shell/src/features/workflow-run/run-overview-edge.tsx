import { memo } from "react";
import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  type EdgeProps,
} from "@xyflow/react";
import { cn } from "@ora/ui";

interface RunOverviewEdgeData extends Record<string, unknown> {
  /** True when the edge is on the executed path. */
  activePath?: boolean;
}

/**
 * Read-only edge: quieter than the settings editor, brighter on the active path.
 */
export const RunOverviewEdge = memo(function RunOverviewEdge({
  id,
  sourceX,
  sourceY,
  sourcePosition,
  targetX,
  targetY,
  targetPosition,
  label,
  markerEnd,
  style,
  data,
}: EdgeProps) {
  const [edgePath, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
  });
  const edgeData = data as RunOverviewEdgeData | undefined;
  const active = edgeData?.activePath === true;
  const stroke = active
    ? "color-mix(in oklch, var(--foreground) 62%, transparent)"
    : "color-mix(in oklch, var(--foreground) 28%, transparent)";

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        markerEnd={markerEnd}
        style={{
          ...style,
          strokeWidth: active ? 2.25 : 1.5,
          stroke,
          opacity: active ? 1 : 0.55,
        }}
      />
      <EdgeLabelRenderer>
        {label !== undefined && label !== null && label !== "" && (
          <div
            className={cn(
              "nodrag nopan pointer-events-none absolute text-[10px]",
              active ? "text-muted-foreground" : "text-muted-foreground/70",
            )}
            style={{
              transform: `translate(-50%, -50%) translate(${labelX}px,${labelY}px)`,
            }}
          >
            {label}
          </div>
        )}
      </EdgeLabelRenderer>
    </>
  );
});
