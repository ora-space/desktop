import { memo } from "react";
import { useTranslation } from "react-i18next";
import { MiniMap, type Node } from "@xyflow/react";
import type { WorkflowNodeData } from "@ora/workflow-mock";

/** Resolves minimap node color without allocating a new callback on canvas renders. */
function workflowOverviewNodeColor(node: Node<WorkflowNodeData>): string {
  if (node.selected) {
    return "var(--ring)";
  }
  return "color-mix(in oklch, var(--foreground) 45%, var(--muted))";
}

/** Shows an interactive graph overview when multiple nodes benefit from spatial navigation. */
export const WorkflowFlowOverview = memo(function WorkflowFlowOverview({
  nodeCount,
}: {
  nodeCount: number;
}) {
  const { t } = useTranslation();

  if (nodeCount < 2) {
    return null;
  }

  return (
    <MiniMap
      ariaLabel={t("settings.workflow.minimap")}
      bgColor="var(--background)"
      maskColor="color-mix(in oklch, var(--background) 72%, transparent)"
      maskStrokeColor="color-mix(in oklch, var(--foreground) 22%, transparent)"
      nodeBorderRadius={8}
      nodeColor={workflowOverviewNodeColor}
      nodeStrokeColor="var(--background)"
      nodeStrokeWidth={2}
      pannable
      position="top-right"
      zoomable
    />
  );
});
