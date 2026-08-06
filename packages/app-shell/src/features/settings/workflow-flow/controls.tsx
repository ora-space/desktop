import { useTranslation } from "react-i18next";
import { useReactFlow, useViewport } from "@xyflow/react";
import {
  IconArrowsMaximize,
  IconFocusCentered,
  IconLayoutSidebarLeftExpand,
  IconLayoutSidebarRightExpand,
  IconMinus,
  IconPlus,
} from "@tabler/icons-react";
import { Button } from "@ora/ui";
import {
  MAX_WORKFLOW_ZOOM,
  MIN_WORKFLOW_ZOOM,
} from "./viewport";

interface WorkflowCanvasControlsProps {
  defaultViewport: { x: number; y: number; zoom: number };
  libraryCollapsed: boolean;
  inspectorCollapsed: boolean;
  inspectorAvailable: boolean;
  onExpandLibrary: () => void;
  onExpandInspector: () => void;
}

/** Renders React Flow viewport controls and the contextual panel restore actions. */
export function WorkflowCanvasControls({
  defaultViewport,
  libraryCollapsed,
  inspectorCollapsed,
  inspectorAvailable,
  onExpandLibrary,
  onExpandInspector,
}: WorkflowCanvasControlsProps) {
  const { t } = useTranslation();
  const { fitView, setViewport, zoomTo } = useReactFlow();
  const { zoom } = useViewport();

  return (
    <>
      <div
        data-workflow-controls
        className="pointer-events-auto absolute right-2 top-2 z-30 flex w-fit items-center rounded-lg border border-border/80 bg-background/95 p-px shadow-sm backdrop-blur"
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

      {(libraryCollapsed || inspectorCollapsed) && (
        <div
          data-workflow-controls
          className="pointer-events-auto absolute left-2 top-2 z-30 flex items-center gap-px rounded-lg border border-border/80 bg-background/95 p-px shadow-sm backdrop-blur"
        >
          {libraryCollapsed && (
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-7 rounded-md"
              aria-label={t("settings.workflow.expandLibrary")}
              onClick={onExpandLibrary}
            >
              <IconLayoutSidebarLeftExpand />
            </Button>
          )}
          {inspectorCollapsed && inspectorAvailable && (
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-7 rounded-md"
              aria-label={t("settings.workflow.expandConfiguration")}
              onClick={onExpandInspector}
            >
              <IconLayoutSidebarRightExpand />
            </Button>
          )}
        </div>
      )}
    </>
  );
}
