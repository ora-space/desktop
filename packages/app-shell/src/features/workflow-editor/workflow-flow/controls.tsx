import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  useReactFlow,
  useStore,
  useStoreApi,
  useViewport,
} from "@xyflow/react";
import {
  IconArrowsMaximize,
  IconFocusCentered,
  IconLayoutSidebarRightExpand,
  IconMinus,
  IconPlus,
} from "@tabler/icons-react";
import { Button } from "@ora/ui";
import {
  MAX_WORKFLOW_ZOOM,
  MIN_WORKFLOW_ZOOM,
} from "../../workflow-node-chrome/viewport";
import { unionScreenBox, viewportDeltaToCenter } from "./scroll-geometry";

interface WorkflowCanvasControlsProps {
  defaultViewport: { x: number; y: number; zoom: number };
}

interface WorkflowCanvasInspectorRestoreProps {
  onExpandInspector: () => void;
}

/**
 * Pans the current zoom so the selected card sits in the middle of the canvas.
 * Measures the rendered card, because iteration members are drawn inside a
 * parent frame and their graph position is not the on-screen position.
 */
export function WorkflowScrollSelectedNode({
  onRevealHiddenSelection,
}: {
  /** Expands collapsed frames that are hiding the cards about to be centered. */
  onRevealHiddenSelection?: (ids: readonly string[]) => void;
}) {
  const { t } = useTranslation();
  const selectedIds = useStore((state) =>
    state.nodes
      .filter((node) => node.selected)
      .map((node) => node.id)
      .join("\0"),
  );
  const { getViewport, setViewport } = useReactFlow();
  const store = useStoreApi();
  // Closing the right-hand inspector clears the selection. Keep the last card
  // so the button can still bring it back. Updating state during render is the
  // supported way to remember a previous value without reading a ref.
  const [rememberedIds, setRememberedIds] = useState<string[]>([]);
  const liveIds = selectedIds === "" ? [] : selectedIds.split("\0");
  if (liveIds.length > 0 && rememberedIds.join("\0") !== selectedIds) {
    setRememberedIds(liveIds);
  }
  const ids = liveIds.length > 0 ? liveIds : rememberedIds;

  /** Pans to the rendered cards. Returns false when none are measurable yet. */
  const panToIds = (targetIds: readonly string[]): boolean => {
    const boxes = targetIds.flatMap((id) => {
      const card = document.querySelector(
        `[data-workflow-node-id="${CSS.escape(id)}"]`,
      );
      if (!(card instanceof HTMLElement)) {
        return [];
      }
      const rect = card.getBoundingClientRect();
      if (rect.width === 0 && rect.height === 0) {
        return [];
      }
      return [
        {
          left: rect.left,
          top: rect.top,
          width: rect.width,
          height: rect.height,
        },
      ];
    });
    const target = unionScreenBox(boxes);
    const pane = document.querySelector(".workflow-flow");
    if (target === null || !(pane instanceof HTMLElement)) {
      return false;
    }
    const paneRect = pane.getBoundingClientRect();
    const delta = viewportDeltaToCenter(
      {
        left: paneRect.left,
        top: paneRect.top,
        width: paneRect.width,
        height: paneRect.height,
      },
      target,
    );
    const viewport = getViewport();
    void setViewport(
      {
        x: viewport.x + delta.x,
        y: viewport.y + delta.y,
        zoom: viewport.zoom,
      },
      { duration: 220 },
    );
    return true;
  };

  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      className="nopan nodrag nowheel relative z-50 h-7 rounded-full bg-background/95 px-3 text-xs shadow-sm backdrop-blur"
      disabled={ids.length === 0}
      title={ids.length === 0 ? t("settings.workflow.noSelection") : undefined}
      onPointerDown={(event) => {
        event.stopPropagation();
      }}
      onClick={() => {
        if (liveIds.length === 0) {
          store.getState().addSelectedNodes(ids);
        }
        const retry = (attempt: number): void => {
          if (panToIds(ids)) {
            return;
          }
          if (attempt === 0) {
            onRevealHiddenSelection?.(ids);
          }
          if (attempt >= 12) {
            return;
          }
          requestAnimationFrame(() => {
            retry(attempt + 1);
          });
        };
        retry(0);
      }}
    >
      {t("settings.workflow.scrollToSelectedNode")}
    </Button>
  );
}

/** Renders a compact Dify-style zoom group anchored to the canvas bottom-right. */
export function WorkflowCanvasControls({
  defaultViewport,
}: WorkflowCanvasControlsProps) {
  const { t } = useTranslation();
  const { fitView, setViewport, zoomTo } = useReactFlow();
  const { zoom } = useViewport();

  return (
    <div
      data-workflow-controls
      data-workflow-viewport-controls
      className="pointer-events-auto absolute bottom-3 right-3 z-40 flex w-fit items-center gap-0.5 rounded-xl border border-border bg-background/95 p-1 shadow-lg backdrop-blur"
      aria-label={t("settings.workflow.canvasControls")}
      aria-orientation="horizontal"
      role="toolbar"
    >
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-8 rounded-lg"
        aria-label={t("settings.workflow.zoomOut")}
        disabled={zoom <= MIN_WORKFLOW_ZOOM}
        onClick={() => {
          void zoomTo(zoom - 0.1);
        }}
      >
        <IconMinus />
      </Button>
      <span
        className="flex h-8 min-w-11 items-center justify-center px-1 text-[10px] font-medium tabular-nums text-muted-foreground"
        aria-live="polite"
      >
        {Math.round(zoom * 100)}%
      </span>
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-8 rounded-lg"
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
        className="size-8 rounded-lg"
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
        className="size-8 rounded-lg"
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
