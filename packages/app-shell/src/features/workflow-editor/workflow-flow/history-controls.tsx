import { useEffect, useMemo, useRef, useState, type SVGProps } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@ora/ui";
import type {
  WorkflowHistoryEvent,
  WorkflowHistoryStep,
} from "../workflow-history";

interface WorkflowHistoryControlsProps {
  canUndo: boolean;
  canRedo: boolean;
  past: WorkflowHistoryStep[];
  future: WorkflowHistoryStep[];
  currentEvent: WorkflowHistoryEvent | null;
  currentMeta?: WorkflowHistoryStep["meta"];
  readOnly: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onJump: (direction: "past" | "future", steps: number) => void;
  onClear: () => void;
}

type HistoryRow = {
  id: string;
  direction: "current" | "past" | "future";
  steps: number;
  event: WorkflowHistoryEvent | null;
  meta?: WorkflowHistoryStep["meta"];
};

/** Draws the Lucide Undo path without adding another icon package to app-shell. */
function LucideUndo(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      {...props}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M9 14 4 9l5-5" />
      <path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5v0a5.5 5.5 0 0 1-5.5 5.5H11" />
    </svg>
  );
}

/** Draws the Lucide Redo path without adding another icon package to app-shell. */
function LucideRedo(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      {...props}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="m15 14 5-5-5-5" />
      <path d="M20 9H9.5A5.5 5.5 0 0 0 4 14.5v0A5.5 5.5 0 0 0 9.5 20H13" />
    </svg>
  );
}

/** Draws the requested Lucide List Clock icon without adding another package. */
function LucideListClock(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      {...props}
      width="24"
      height="24"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`lucide lucide-list-clock-icon lucide-list-clock ${props.className ?? ""}`}
      aria-hidden="true"
    >
      <path d="M16 13v2.2l1.6 1" />
      <path d="M3 12h3.458" />
      <path d="M3 19h3.832" />
      <path d="M3 5h18" />
      <circle cx="16" cy="15" r="6" />
    </svg>
  );
}

/** Renders the session undo/redo buttons and the Dify-inspired change history list. */
export function WorkflowHistoryControls({
  canUndo,
  canRedo,
  past,
  future,
  currentEvent,
  currentMeta,
  readOnly,
  onUndo,
  onRedo,
  onJump,
  onClear,
}: WorkflowHistoryControlsProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const historyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    /** Dismisses the panel when focus moves back to the canvas or surrounding editor. */
    function dismissOnOutsidePointerDown(event: PointerEvent): void {
      const target = event.target;
      if (target instanceof Node && !historyRef.current?.contains(target)) {
        setOpen(false);
      }
    }
    /** Supports the familiar keyboard dismissal path without adding another visible control. */
    function dismissOnEscape(event: KeyboardEvent): void {
      if (event.key === "Escape") {
        setOpen(false);
      }
    }
    window.addEventListener("pointerdown", dismissOnOutsidePointerDown);
    window.addEventListener("keydown", dismissOnEscape);
    return () => {
      window.removeEventListener("pointerdown", dismissOnOutsidePointerDown);
      window.removeEventListener("keydown", dismissOnEscape);
    };
  }, [open]);

  const rows = useMemo<HistoryRow[]>(
    () => [
      ...future.map((step, index) => ({
        id: `future-${step.id}`,
        direction: "future" as const,
        // Future is newest-first because each undo appends the operation that
        // was just left. Keep that order so the panel stays newest-to-oldest.
        steps: future.length - index,
        event: step.event,
        meta: step.meta,
      })),
      {
        id: "current",
        direction: "current",
        steps: 0,
        event: currentEvent,
        meta: currentMeta,
      },
      ...past
        .slice()
        .reverse()
        .map((step, index) => ({
          id: `past-${step.id}`,
          direction: "past" as const,
          steps: index + 1,
          // A past snapshot is the state before its own edit, so its label is
          // the preceding edit (or the session baseline for the oldest row).
          event: past[past.length - index - 2]?.event ?? null,
          meta: past[past.length - index - 2]?.meta,
        })),
    ],
    [currentEvent, currentMeta, future, past],
  );

  /** Converts an internal event into stable, localized product language. */
  function eventLabel(
    event: WorkflowHistoryEvent | null,
    meta?: WorkflowHistoryStep["meta"],
  ): string {
    if (event === null) {
      return t("settings.workflow.historySessionStart");
    }
    const labels: Record<WorkflowHistoryEvent, string> = {
      "node.add": t("settings.workflow.historyEventNodeAdd"),
      "annotation.add": t("settings.workflow.historyEventAnnotationAdd"),
      "node.delete": t("settings.workflow.historyEventNodeDelete"),
      "edge.delete": t("settings.workflow.historyEventEdgeDelete"),
      "edge.connect": t("settings.workflow.historyEventEdgeConnect"),
      "edge.reconnect": t("settings.workflow.historyEventEdgeReconnect"),
      "node.move": t("settings.workflow.historyEventNodeMove"),
      "layout.organize": t("settings.workflow.historyEventOrganize"),
      "node.edit": t("settings.workflow.historyEventNodeEdit"),
      "annotation.edit": t("settings.workflow.historyEventAnnotationEdit"),
      "workflow.variables": t("settings.workflow.historyEventVariables"),
      "workflow.rename": t("settings.workflow.historyEventRename"),
    };
    const subject = meta?.subject ?? meta?.nodeTitle;
    if (subject !== undefined && subject !== "") {
      return `${labels[event]}：${subject}`;
    }
    return labels[event];
  }

  /** Handles a row click while keeping the current state row non-interactive. */
  function selectRow(row: HistoryRow): void {
    if (readOnly || row.direction === "current") {
      return;
    }
    onJump(row.direction, row.steps);
  }

  return (
    <div
      ref={historyRef}
      className="pointer-events-auto relative flex items-center rounded-lg border border-border bg-background/95 p-1 shadow-sm backdrop-blur"
    >
      <Button
        variant="ghost"
        size="icon-sm"
        disabled={readOnly || !canUndo}
        aria-label={t("settings.workflow.undo")}
        title={t("settings.workflow.undo")}
        onClick={onUndo}
      >
        <LucideUndo className="size-4" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        disabled={readOnly || !canRedo}
        aria-label={t("settings.workflow.redo")}
        title={t("settings.workflow.redo")}
        onClick={onRedo}
      >
        <LucideRedo className="size-4" />
      </Button>
      <div className="mx-1 h-5 w-px bg-border" aria-hidden />
      <Button
        variant={open ? "secondary" : "ghost"}
        size="icon-sm"
        disabled={readOnly && past.length === 0 && future.length === 0}
        aria-label={t("settings.workflow.changeHistory")}
        title={t("settings.workflow.changeHistory")}
        onClick={() => setOpen((value) => !value)}
      >
        <LucideListClock className="size-4" />
      </Button>
      {open && (
        <div className="absolute bottom-11 left-0 z-50 w-72 overflow-hidden rounded-xl border border-border bg-background shadow-xl">
          <div className="border-b border-border px-3 py-2">
            <h3 className="text-xs font-medium">
              {t("settings.workflow.changeHistory")}
            </h3>
          </div>
          <div className="max-h-72 overflow-y-auto p-1.5">
            {rows.map((row) => {
              const current = row.direction === "current";
              const label = eventLabel(row.event, row.meta);
              const description = current
                ? t("settings.workflow.historyCurrent")
                : row.direction === "past"
                  ? t("settings.workflow.historyStepsBack", {
                      count: row.steps,
                    })
                  : t("settings.workflow.historyStepsForward", {
                      count: row.steps,
                    });
              const rowLabel = `${label}（${description}）`;
              return (
                <button
                  key={row.id}
                  type="button"
                  className={`flex w-full flex-col items-start rounded-md px-2 py-1.5 text-left text-xs transition-colors ${
                    current ? "bg-muted font-medium" : "hover:bg-muted/70"
                  }`}
                  disabled={readOnly || current}
                  onClick={() => selectRow(row)}
                  title={rowLabel}
                >
                  <span className="truncate">{rowLabel}</span>
                </button>
              );
            })}
          </div>
          <div className="border-t border-border p-1.5">
            <Button
              variant="ghost"
              size="sm"
              className="w-full justify-start text-xs"
              disabled={readOnly || (!canUndo && !canRedo)}
              onClick={onClear}
            >
              {t("settings.workflow.historyClear")}
            </Button>
            <p className="px-2 pb-1 pt-2 text-[10px] leading-4 text-muted-foreground">
              {t("settings.workflow.historyHint")}
            </p>
          </div>
        </div>
      )}
    </div>
  );
}
