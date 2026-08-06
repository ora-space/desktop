import { memo } from "react";
import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  type EdgeProps,
} from "@xyflow/react";
import { cn } from "@ora/ui";

/** Draws a selectable workflow edge with an accessible hit target and optional branch label. */
export const WorkflowFlowEdgeView = memo(function WorkflowFlowEdgeView({
  id,
  sourceX,
  sourceY,
  sourcePosition,
  targetX,
  targetY,
  targetPosition,
  label,
  selected,
  markerEnd,
  style,
  interactionWidth,
}: EdgeProps) {
  const [edgePath, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
  });
  const edgeColor = selected
    ? "var(--ring)"
    : "color-mix(in oklch, var(--foreground) 46%, transparent)";

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        markerEnd={markerEnd}
        interactionWidth={interactionWidth}
        style={{
          ...style,
          strokeWidth: selected ? 3 : 2,
          stroke: edgeColor,
        }}
      />
      {selected && (
        <g className="pointer-events-none">
          {[
            { x: sourceX, y: sourceY },
            { x: targetX, y: targetY },
          ].map((endpoint) => (
            <g key={`${endpoint.x}-${endpoint.y}`}>
              <circle
                cx={endpoint.x}
                cy={endpoint.y}
                r={8}
                fill="var(--background)"
                stroke="var(--ring)"
                strokeWidth={2}
              />
              <circle
                cx={endpoint.x}
                cy={endpoint.y}
                r={4.5}
                fill="var(--foreground)"
              />
            </g>
          ))}
        </g>
      )}
      <EdgeLabelRenderer>
        {label !== undefined && label !== null && label !== "" && (
          <div
            className={cn(
              "nodrag nopan pointer-events-none absolute text-[10px] text-muted-foreground",
              selected && "text-foreground",
            )}
            style={{
              transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY - 14}px)`,
            }}
          >
            {String(label)}
          </div>
        )}
      </EdgeLabelRenderer>
    </>
  );
});
