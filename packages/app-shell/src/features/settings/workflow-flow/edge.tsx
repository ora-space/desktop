import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  type EdgeProps,
} from "@xyflow/react";
import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import { useWorkflowFlowCallbacks } from "./callbacks";

/** Draws a selectable bezier edge with an accessible hit target and optional branch label. */
export function WorkflowFlowEdgeView({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  label,
  selected,
  markerEnd,
  style,
  data,
}: EdgeProps) {
  const { t } = useTranslation();
  const { onDeleteEdge, onSelectEdge } = useWorkflowFlowCallbacks();
  const [edgePath, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  });
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
          stroke: selected
            ? "var(--ring)"
            : "color-mix(in oklch, var(--foreground) 34%, transparent)",
        }}
      />
      <path
        data-workflow-edge={id}
        d={edgePath}
        fill="none"
        stroke="transparent"
        strokeWidth={16}
        className="react-flow__edge-interaction"
      />
      <EdgeLabelRenderer>
        <button
          type="button"
          className={cn(
            "nodrag nopan absolute size-6 -translate-x-1/2 -translate-y-1/2 rounded-full",
            selected ? "bg-ring/25" : "bg-transparent",
          )}
          style={{
            transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
          }}
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
            if (event.key === "Delete" || event.key === "Backspace") {
              event.preventDefault();
              event.stopPropagation();
              onDeleteEdge(id);
            }
          }}
        />
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
}
