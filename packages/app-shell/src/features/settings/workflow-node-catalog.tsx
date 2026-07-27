import {
  useEffect,
  useRef,
  useState,
  type WheelEvent as ReactWheelEvent,
} from "react";
import { IconAdjustmentsAlt } from "@tabler/icons-react";
import type { WorkflowNodeKind } from "@ora/workflow-mock";
import { cn } from "@ora/ui";
import { useTranslation } from "react-i18next";
import {
  WORKFLOW_NODE_CATALOG,
  WORKFLOW_NODE_DRAG_DATA_TYPE,
} from "./workflow-node-metadata";

const MAX_ELASTIC_OFFSET = 18;
const ELASTIC_RESISTANCE = 0.14;
const WHEEL_END_DELAY_MS = 100;

/** Presents node types as a compact bottom dock that stays close to the canvas. */
export function WorkflowNodeCatalog({
  onAdd,
}: {
  onAdd: (kind: WorkflowNodeKind) => void;
}) {
  const { t } = useTranslation();
  const scrollViewportRef = useRef<HTMLDivElement>(null);
  const returnTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const elasticOffsetRef = useRef(0);
  const [elasticOffset, setElasticOffset] = useState(0);
  const [returning, setReturning] = useState(false);

  useEffect(
    () => () => {
      if (returnTimerRef.current !== null) {
        clearTimeout(returnTimerRef.current);
      }
    },
    [],
  );

  /** Converts wheel motion into horizontal scrolling and adds bounded edge resistance. */
  function handleWheel(event: ReactWheelEvent<HTMLDivElement>): void {
    event.stopPropagation();
    const viewport = scrollViewportRef.current;
    if (viewport === null) {
      return;
    }

    const maxScrollLeft = Math.max(0, viewport.scrollWidth - viewport.clientWidth);
    event.preventDefault();
    const dominantDelta = Math.abs(event.deltaX) > Math.abs(event.deltaY)
      ? event.deltaX
      : event.deltaY;
    const deltaMultiplier = event.deltaMode === WheelEvent.DOM_DELTA_LINE
      ? 16
      : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
        ? viewport.clientWidth
        : 1;
    const requestedScrollLeft = viewport.scrollLeft + dominantDelta * deltaMultiplier;
    const nextScrollLeft = Math.min(maxScrollLeft, Math.max(0, requestedScrollLeft));
    const overflow = requestedScrollLeft - nextScrollLeft;
    viewport.scrollLeft = nextScrollLeft;

    if (returnTimerRef.current !== null) {
      clearTimeout(returnTimerRef.current);
    }

    const reduceMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
    const nextElasticOffset = reduceMotion
      ? 0
      : Math.min(
          MAX_ELASTIC_OFFSET,
          Math.max(
            -MAX_ELASTIC_OFFSET,
            elasticOffsetRef.current - overflow * ELASTIC_RESISTANCE,
          ),
        );
    elasticOffsetRef.current = nextElasticOffset;
    setReturning(false);
    setElasticOffset(nextElasticOffset);

    returnTimerRef.current = setTimeout(() => {
      elasticOffsetRef.current = 0;
      setReturning(true);
      setElasticOffset(0);
    }, WHEEL_END_DELAY_MS);
  }

  return (
    <div
      className="flex w-max max-w-full items-center gap-1.5 rounded-xl border border-border bg-background/95 p-1.5 shadow-lg backdrop-blur"
      aria-label={t("settings.workflow.addNode")}
      aria-orientation="horizontal"
      onWheel={handleWheel}
      role="toolbar"
    >
      <div className="hidden shrink-0 items-center gap-1.5 border-r border-border px-2 xl:flex">
        <IconAdjustmentsAlt className="size-3.5 text-muted-foreground" />
        <span className="text-[10px] font-semibold">{t("settings.workflow.nodes")}</span>
      </div>
      <div
        ref={scrollViewportRef}
        data-workflow-node-scroll
        className="min-w-0 overflow-x-auto overscroll-x-contain [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        <div
          data-workflow-node-track
          className={cn(
            "flex w-max items-center gap-1 px-1",
            returning && "transition-transform duration-200 ease-out motion-reduce:transition-none",
          )}
          style={{ transform: `translate3d(${elasticOffset}px, 0, 0)` }}
        >
          {WORKFLOW_NODE_CATALOG.map((item) => {
            const Icon = item.icon;
            return (
              <button
                key={item.kind}
                type="button"
                draggable
                onClick={() => onAdd(item.kind)}
                onDragStart={(event) => {
                  event.dataTransfer.effectAllowed = "copy";
                  event.dataTransfer.setData(WORKFLOW_NODE_DRAG_DATA_TYPE, item.kind);
                }}
                title={`${t(item.descriptionKey)} · ${t("settings.workflow.dragNodeHint")}`}
                className="group flex h-10 shrink-0 cursor-grab items-center gap-1.5 rounded-lg border border-transparent px-2 text-left outline-none transition-colors hover:border-border hover:bg-muted/65 focus-visible:ring-2 focus-visible:ring-ring active:cursor-grabbing"
              >
                <span className={cn("flex size-7 shrink-0 items-center justify-center rounded-md", item.tone)}>
                  <Icon className="size-3.5" stroke={1.8} />
                </span>
                <span className="text-[10px] font-medium">{t(item.labelKey)}</span>
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
