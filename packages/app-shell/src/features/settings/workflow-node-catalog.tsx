import {
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";
import { createPortal } from "react-dom";
import { IconAdjustmentsAlt } from "@tabler/icons-react";
import type { WorkflowNodeKind } from "@ora/workflow-mock";
import { cn } from "@ora/ui";
import { useTranslation } from "react-i18next";
import {
  getNodeMetadata,
  WORKFLOW_NODE_CATALOG,
} from "./workflow-node-metadata";

const MAX_ELASTIC_OFFSET = 18;
const ELASTIC_RESISTANCE = 0.14;
const WHEEL_END_DELAY_MS = 100;
const NODE_DRAG_THRESHOLD = 4;

interface ClientPosition {
  clientX: number;
  clientY: number;
}

interface NodeDragDraft {
  kind: WorkflowNodeKind;
  pointerId: number;
  origin: ClientPosition;
  moved: boolean;
}

interface NodeDragPreview {
  kind: WorkflowNodeKind;
  position: ClientPosition;
}

/** Presents node types as a compact bottom dock that stays close to the canvas. */
export function WorkflowNodeCatalog({
  onAdd,
  onDrop,
}: {
  onAdd: (kind: WorkflowNodeKind) => void;
  onDrop: (kind: WorkflowNodeKind, position: ClientPosition) => void;
}) {
  const { t } = useTranslation();
  const scrollViewportRef = useRef<HTMLDivElement>(null);
  const returnTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const elasticOffsetRef = useRef(0);
  const nodeDragRef = useRef<NodeDragDraft | null>(null);
  const suppressClickRef = useRef(false);
  const [elasticOffset, setElasticOffset] = useState(0);
  const [returning, setReturning] = useState(false);
  const [nodeDragPreview, setNodeDragPreview] = useState<NodeDragPreview | null>(null);

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

  /** Starts a catalog drag with pointer capture because native HTML drag is unavailable in desktop WebViews. */
  function startNodeDrag(
    event: ReactPointerEvent<HTMLButtonElement>,
    kind: WorkflowNodeKind,
  ): void {
    if (event.button !== 0 || !event.isPrimary) {
      return;
    }
    suppressClickRef.current = false;
    setNodeDragPreview(null);
    nodeDragRef.current = {
      kind,
      pointerId: event.pointerId,
      origin: { clientX: event.clientX, clientY: event.clientY },
      moved: false,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  /** Distinguishes an intentional drag from a click without interfering with catalog activation. */
  function moveNodeDrag(event: ReactPointerEvent<HTMLButtonElement>): void {
    const draft = nodeDragRef.current;
    if (draft === null || draft.pointerId !== event.pointerId) {
      return;
    }
    if (
      !draft.moved
      && Math.hypot(
        event.clientX - draft.origin.clientX,
        event.clientY - draft.origin.clientY,
      ) < NODE_DRAG_THRESHOLD
    ) {
      return;
    }
    draft.moved = true;
    setNodeDragPreview({
      kind: draft.kind,
      position: { clientX: event.clientX, clientY: event.clientY },
    });
  }

  /** Drops a moved node at the release coordinates and suppresses the synthetic click that follows. */
  function finishNodeDrag(event: ReactPointerEvent<HTMLButtonElement>): void {
    const draft = nodeDragRef.current;
    if (draft === null || draft.pointerId !== event.pointerId) {
      return;
    }
    nodeDragRef.current = null;
    setNodeDragPreview(null);
    const moved = draft.moved || Math.hypot(
      event.clientX - draft.origin.clientX,
      event.clientY - draft.origin.clientY,
    ) >= NODE_DRAG_THRESHOLD;
    if (!moved) {
      return;
    }
    suppressClickRef.current = true;
    event.preventDefault();
    onDrop(draft.kind, { clientX: event.clientX, clientY: event.clientY });
  }

  /** Clears an interrupted pointer gesture so the next click remains independent. */
  function cancelNodeDrag(event: ReactPointerEvent<HTMLButtonElement>): void {
    if (nodeDragRef.current?.pointerId === event.pointerId) {
      nodeDragRef.current = null;
      setNodeDragPreview(null);
    }
  }

  /** Preserves keyboard and pointer click-to-add while ignoring the click emitted after a drag. */
  function addNodeFromClick(
    event: ReactMouseEvent<HTMLButtonElement>,
    kind: WorkflowNodeKind,
  ): void {
    if (suppressClickRef.current) {
      suppressClickRef.current = false;
      event.preventDefault();
      return;
    }
    onAdd(kind);
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
                onClick={(event) => addNodeFromClick(event, item.kind)}
                onPointerDown={(event) => startNodeDrag(event, item.kind)}
                onPointerMove={moveNodeDrag}
                onPointerUp={finishNodeDrag}
                onPointerCancel={cancelNodeDrag}
                onLostPointerCapture={cancelNodeDrag}
                title={`${t(item.descriptionKey)} · ${t("settings.workflow.dragNodeHint")}`}
                className="group flex h-10 shrink-0 touch-none cursor-grab items-center gap-1.5 rounded-lg border border-transparent px-2 text-left outline-none transition-colors hover:border-border hover:bg-muted/65 focus-visible:ring-2 focus-visible:ring-ring active:cursor-grabbing"
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
      {nodeDragPreview !== null && createPortal(
        <WorkflowNodeDragPreview {...nodeDragPreview} />,
        document.body,
      )}
    </div>
  );
}

/** Renders a non-interactive node capsule centered on the pointer during a catalog drag. */
function WorkflowNodeDragPreview({
  kind,
  position,
}: NodeDragPreview) {
  const { t } = useTranslation();
  const metadata = getNodeMetadata(kind);
  const Icon = metadata.icon;

  return (
    <div
      data-workflow-node-preview
      aria-hidden="true"
      className="pointer-events-none fixed z-[100] flex h-10 items-center gap-1.5 rounded-full border border-foreground/20 bg-background/95 px-2 shadow-lg backdrop-blur-sm"
      style={{
        left: position.clientX,
        top: position.clientY,
        transform: "translate(-50%, -50%)",
      }}
    >
      <span className={cn(
        "flex size-7 shrink-0 items-center justify-center rounded-full",
        metadata.tone,
      )}>
        <Icon className="size-3.5" stroke={1.8} />
      </span>
      <span className="pr-1 text-[10px] font-medium">{t(metadata.labelKey)}</span>
    </div>
  );
}
