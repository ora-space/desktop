import { useTranslation } from "react-i18next";
import {
  Grid2x2CheckIcon,
  HandIcon,
  MousePointer2Icon,
  StickyNotePlusIcon,
} from "./canvas-tool-icons";
import { Button } from "@ora/ui";

export type CanvasInteractionMode = "pointer" | "hand";

interface WorkflowCanvasToolsProps {
  mode: CanvasInteractionMode;
  readOnly: boolean;
  onModeChange: (mode: CanvasInteractionMode) => void;
  onAddAnnotation: () => void;
  onOrganize: () => void;
}

/** Renders creation, interaction-mode, and layout tools beside the canvas. */
export function WorkflowCanvasTools({
  mode,
  readOnly,
  onModeChange,
  onAddAnnotation,
  onOrganize,
}: WorkflowCanvasToolsProps) {
  const { t } = useTranslation();
  return (
    <div
      data-workflow-tools
      className="pointer-events-auto absolute left-2 top-1/2 z-30 flex -translate-y-1/2 flex-col items-center rounded-xl border border-border/80 bg-background/95 p-1 shadow-sm backdrop-blur"
      aria-label={t("settings.workflow.canvasTools")}
      aria-orientation="vertical"
      role="toolbar"
    >
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-8 rounded-lg"
        aria-label={t("settings.workflow.addAnnotation")}
        title={t("settings.workflow.addAnnotation")}
        disabled={readOnly}
        onClick={onAddAnnotation}
      >
        <StickyNotePlusIcon />
      </Button>
      <span className="my-1 h-px w-5 bg-border" aria-hidden="true" />
      <Button
        variant={mode === "pointer" ? "secondary" : "ghost"}
        size="icon-sm"
        className="size-8 rounded-lg"
        aria-label={t("settings.workflow.pointerMode")}
        aria-pressed={mode === "pointer"}
        title={t("settings.workflow.pointerMode")}
        onClick={() => onModeChange("pointer")}
      >
        <MousePointer2Icon />
      </Button>
      <Button
        variant={mode === "hand" ? "secondary" : "ghost"}
        size="icon-sm"
        className="size-8 rounded-lg"
        aria-label={t("settings.workflow.handMode")}
        aria-pressed={mode === "hand"}
        title={t("settings.workflow.handMode")}
        onClick={() => onModeChange("hand")}
      >
        <HandIcon />
      </Button>
      <span className="my-1 h-px w-5 bg-border" aria-hidden="true" />
      <Button
        variant="ghost"
        size="icon-sm"
        className="size-8 rounded-lg"
        aria-label={t("settings.workflow.organizeNodes")}
        title={t("settings.workflow.organizeNodes")}
        disabled={readOnly}
        onClick={onOrganize}
      >
        <Grid2x2CheckIcon />
      </Button>
    </div>
  );
}
