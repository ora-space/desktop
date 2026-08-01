import { memo } from "react";
import {
  BaseEdge,
  EdgeLabelRenderer,
  type EdgeProps,
} from "@xyflow/react";
import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import { useWorkflowFlowActions } from "./callbacks";
import { workflowEdgePath } from "./path";

/** Draws a selectable workflow edge with an accessible hit target and optional branch label. */
export const WorkflowFlowEdgeView = memo(function WorkflowFlowEdgeView({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  label,
  selected,
  markerEnd,
  style,
  data,
}: EdgeProps) {
  const { t } = useTranslation();
  const { onDeleteEdge, onSelectEdge } = useWorkflowFlowActions();
  const edgePath = workflowEdgePath({
    sourceX,
    sourceY,
    targetX,
    targetY,
  });
  const labelX = (sourceX + targetX) / 2;
  const labelY = (sourceY + targetY) / 2;
  const edgeColor = selected
    ? "var(--ring)"
    : "color-mix(in oklch, var(--foreground) 46%, transparent)";
  const sourceTitle = typeof data?.sourceTitle === "string" ? data.sourceTitle : "";
  const targetTitle = typeof data?.targetTitle === "string" ? data.targetTitle : "";
  const accessibleName = t("settings.workflow.selectConnection", {
    source: sourceTitle,
    target: targetTitle,
  });

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        markerEnd={markerEnd}
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
      <path
        data-workflow-edge={id}
        d={edgePath}
        fill="none"
        stroke="transparent"
        strokeWidth={20}
        className="react-flow__edge-interaction cursor-pointer outline-none"
        role="button"
        tabIndex={0}
        aria-label={accessibleName}
        aria-keyshortcuts="Delete Backspace"
        onClick={(event) => {
          event.stopPropagation();
          onSelectEdge(id);
        }}
        onDoubleClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onDeleteEdge(id);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onSelectEdge(id);
          }
          if (event.key === "Delete" || event.key === "Backspace") {
            event.preventDefault();
            onDeleteEdge(id);
          }
        }}
      />
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
