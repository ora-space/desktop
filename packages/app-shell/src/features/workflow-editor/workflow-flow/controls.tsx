import { useTranslation } from "react-i18next";
import { useReactFlow, useViewport } from "@xyflow/react";
import {
  IconArrowsMaximize,
  IconFocusCentered,
  IconLayoutSidebarRightExpand,
  IconMinus,
  IconPlus,
} from "@tabler/icons-react";
import { Button } from "@ora/ui";
import { MAX_WORKFLOW_ZOOM, MIN_WORKFLOW_ZOOM } from "./viewport";

interface WorkflowCanvasControlsProps {
  defaultViewport: { x: number; y: number; zoom: number };
}

interface WorkflowCanvasInspectorRestoreProps {
  onExpandInspector: () => void;
}

/** Renders React Flow viewport zoom and fit controls for the canvas chrome row. */
export function WorkflowCanvasControls({
  defaultViewport,
}: WorkflowCanvasControlsProps) {
  const { t } = useTranslation();
  const { fitView, setViewport, zoomTo } = useReactFlow();
  const { zoom } = useViewport();

  return (
    <div
      data-workflow-controls
      className="flex w-fit items-center rounded-lg border border-border/80 bg-background/95 p-px shadow-sm backdrop-blur"
      aria-label={t("settings.workflow.canvasControls")}
      aria-orientation="horizontal"
      role="toolbar"
    >
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-7 rounded-md"
        aria-label={t("settings.workflow.zoomOut")}
        disabled={zoom <= MIN_WORKFLOW_ZOOM}
        onClick={() => {
          void zoomTo(zoom - 0.1);
        }}
      >
        <IconMinus />
      </Button>
      <span
        className="flex h-7 w-8 items-center justify-center text-[9px] font-medium tabular-nums text-muted-foreground"
        aria-live="polite"
      >
        {Math.round(zoom * 100)}%
      </span>
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-7 rounded-md"
        aria-label={t("settings.workflow.zoomIn")}
        disabled={zoom >= MAX_WORKFLOW_ZOOM}
        onClick={() => {
          void zoomTo(zoom + 0.1);
        }}
      >
        <IconPlus />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-7 rounded-md"
        aria-label={t("settings.workflow.fitView")}
        onClick={() => {
          void fitView({
            duration: 220,
            maxZoom: 1,
            minZoom: MIN_WORKFLOW_ZOOM,
            padding: 0.16,
          });
        }}
      >
        <IconArrowsMaximize />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-7 rounded-md"
        aria-label={t("settings.workflow.resetView")}
        onClick={() => {
          void setViewport(defaultViewport, { duration: 180 });
        }}
      >
        <IconFocusCentered />
      </Button>
      <span className="sr-only">{t("settings.workflow.canvasHint")}</span>
    </div>
  );
}

/** Restores a collapsed inspector from the same top chrome row as zoom and version status. */
export function WorkflowCanvasInspectorRestore({
  onExpandInspector,
}: WorkflowCanvasInspectorRestoreProps) {
  const { t } = useTranslation();

  return (
    <div
      data-workflow-controls
      className="flex items-center gap-px rounded-lg border border-border/80 bg-background/95 p-px shadow-sm backdrop-blur"
    >
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-7 rounded-md"
        aria-label={t("settings.workflow.expandConfiguration")}
        onClick={onExpandInspector}
      >
        <IconLayoutSidebarRightExpand />
      </Button>
    </div>
  );
}
