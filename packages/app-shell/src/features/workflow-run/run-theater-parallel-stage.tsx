import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { IconChevronLeft, IconChevronRight } from "@tabler/icons-react";
import { cn } from "@ora/ui";
import { resolveParallelDragSwitch } from "./parallel-drag";
import { RunTheaterActCard } from "./run-theater-act-card";
import { isNodeWorking } from "./run-status-style";
import type {
  GraphWorkflowNodeState,
  WorkflowNodeData,
  WorkflowNodeConversationItem,
} from "@ora/workflow-runtime";
import "./theater-motion.css";

const DRAG_THRESHOLD_PX = 64;
const CLICK_SLOP_PX = 8;
const SLIDE_MS = 480;
const SNAP_EASE = "cubic-bezier(0.22, 1, 0.36, 1)";

interface ParallelAct {
  nodeId: string;
  data: WorkflowNodeData;
  state: GraphWorkflowNodeState;
  artifactCount: number;
  conversation: WorkflowNodeConversationItem[];
}

interface RunTheaterParallelStageProps {
  runId: string;
  acts: ParallelAct[];
  primaryId: string;
  onFocusNode: (nodeId: string) => void;
  inspectorOpen: boolean;
  onToggleInspector: () => void;
  sessionConversationNodeId?: string | null;
  onSessionConversationNodeIdChange?: (nodeId: string | null) => void;
  onNodeCompleted?: (nodeId: string) => void;
  /**
   * Embedded HITL for the focused parallel act. Peers stay chrome-only so the
   * path chips / dots below remain the switcher.
   */
  primaryInteraction?:
    ReactNode | ((slots: { accessory: ReactNode | null }) => ReactNode);
}

/**
 * Horizontal drag carousel for parallel acts.
 * Chevron / chip / keyboard switches use the same slide tween as drag settle
 * so focus changes don’t hard-cut the track.
 */
export function RunTheaterParallelStage({
  runId,
  acts,
  primaryId,
  onFocusNode,
  inspectorOpen,
  onToggleInspector,
  sessionConversationNodeId = null,
  onSessionConversationNodeIdChange,
  onNodeCompleted,
  primaryInteraction,
}: RunTheaterParallelStageProps) {
  const { t } = useTranslation();
  const trackRef = useRef<HTMLDivElement>(null);
  const pointerIdRef = useRef<number | null>(null);
  const startXRef = useRef(0);
  const draggingRef = useRef(false);
  const slideTimerRef = useRef<number | null>(null);
  const [dragX, setDragX] = useState(0);
  const [dragging, setDragging] = useState(false);
  /** Visual index while a programmed slide is in flight (chevron / chip). */
  const [slideIndex, setSlideIndex] = useState<number | null>(null);

  const committedIndex = Math.max(
    0,
    acts.findIndex((act) => act.nodeId === primaryId),
  );
  const index = slideIndex ?? committedIndex;
  const primary = acts[committedIndex];
  const canGoPrev = committedIndex > 0 && slideIndex === null;
  const canGoNext = committedIndex < acts.length - 1 && slideIndex === null;
  const dragProgress = Math.max(-1, Math.min(1, dragX / 140));

  // External focus (path rail) caught up —drop any stale local slide index.
  // Tracked through the documented render-adjust pattern instead of an effect.
  const [previousFocusState, setPreviousFocusState] = useState({
    committedIndex,
    slideIndex,
  });
  if (
    previousFocusState.committedIndex !== committedIndex ||
    previousFocusState.slideIndex !== slideIndex
  ) {
    setPreviousFocusState({ committedIndex, slideIndex });
    if (slideIndex !== null && committedIndex === slideIndex) {
      setSlideIndex(null);
      setDragX(0);
    }
  }

  useEffect(
    () => () => {
      if (slideTimerRef.current !== null) {
        window.clearTimeout(slideTimerRef.current);
      }
    },
    [],
  );

  function focusAt(nextIndex: number): void {
    const next = acts[nextIndex];
    if (next !== undefined) {
      onFocusNode(next.nodeId);
    }
  }

  /**
   * Programmed peer switch: nudge the track, then commit focus after the tween
   * so chevron clicks feel like the drag settle path —not an instant cut.
   */
  function slideTo(nextIndex: number): void {
    if (
      nextIndex === committedIndex ||
      nextIndex < 0 ||
      nextIndex >= acts.length ||
      slideIndex !== null
    ) {
      return;
    }
    const width = trackRef.current?.clientWidth ?? 360;
    const direction = nextIndex > committedIndex ? -1 : 1;
    if (slideTimerRef.current !== null) {
      window.clearTimeout(slideTimerRef.current);
    }
    setDragging(false);
    setSlideIndex(committedIndex);
    setDragX(direction * Math.min(width * 0.55, width * 0.5));
    requestAnimationFrame(() => {
      setSlideIndex(nextIndex);
      setDragX(0);
      focusAt(nextIndex);
      slideTimerRef.current = window.setTimeout(() => {
        slideTimerRef.current = null;
        setSlideIndex(null);
      }, SLIDE_MS);
    });
  }

  function rubberBand(dx: number): number {
    if (acts.length <= 1) {
      return 0;
    }
    if ((index === 0 && dx > 0) || (index === acts.length - 1 && dx < 0)) {
      return dx * 0.22;
    }
    return dx;
  }

  function onPointerDown(event: ReactPointerEvent<HTMLDivElement>): void {
    if (event.button !== 0 || acts.length <= 1 || slideIndex !== null) {
      return;
    }
    pointerIdRef.current = event.pointerId;
    startXRef.current = event.clientX;
    draggingRef.current = false;
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function onPointerMove(event: ReactPointerEvent<HTMLDivElement>): void {
    if (pointerIdRef.current !== event.pointerId) {
      return;
    }
    const dx = event.clientX - startXRef.current;
    if (!draggingRef.current && Math.abs(dx) >= CLICK_SLOP_PX) {
      draggingRef.current = true;
      setDragging(true);
    }
    if (draggingRef.current) {
      setDragX(rubberBand(dx));
    }
  }

  function finishPointer(event: ReactPointerEvent<HTMLDivElement>): void {
    if (pointerIdRef.current !== event.pointerId) {
      return;
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    pointerIdRef.current = null;

    const dx = event.clientX - startXRef.current;
    const wasDragging = draggingRef.current;
    draggingRef.current = false;

    if (!wasDragging) {
      setDragging(false);
      setDragX(0);
      return;
    }

    const nextIndex = resolveParallelDragSwitch(
      dx,
      DRAG_THRESHOLD_PX,
      committedIndex,
      acts.length,
    );
    const width = trackRef.current?.clientWidth ?? 360;

    if (nextIndex !== null) {
      const direction = nextIndex > committedIndex ? -1 : 1;
      setDragging(false);
      setDragX(direction * Math.min(width * 0.42, Math.abs(dx) + width * 0.18));
      requestAnimationFrame(() => {
        focusAt(nextIndex);
        setDragX(0);
      });
      return;
    }

    setDragging(false);
    setDragX(0);
  }

  function onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>): void {
    if (event.key === "ArrowLeft" && canGoPrev) {
      event.preventDefault();
      slideTo(committedIndex - 1);
      return;
    }
    if (event.key === "ArrowRight" && canGoNext) {
      event.preventDefault();
      slideTo(committedIndex + 1);
      return;
    }
  }

  if (primary === undefined) {
    return null;
  }

  const trackTransition = dragging
    ? "none"
    : `transform ${SLIDE_MS}ms ${SNAP_EASE}`;

  return (
    <div className="w-full">
      <div className="relative">
        <button
          type="button"
          tabIndex={-1}
          disabled={!canGoPrev}
          aria-label={t("workflowRun.theater.parallelPrev")}
          className={cn(
            "absolute left-0 top-1/2 z-20 hidden size-9 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full border border-border/80 bg-background/90 shadow-sm backdrop-blur-sm transition-[opacity,transform] duration-200 sm:flex",
            canGoPrev
              ? "opacity-70 hover:scale-105 hover:opacity-100"
              : "pointer-events-none opacity-0",
            dragging && dragProgress > 0.15 && "scale-110 opacity-100",
          )}
          onClick={() => slideTo(committedIndex - 1)}
        >
          <IconChevronLeft className="size-4" />
        </button>
        <button
          type="button"
          tabIndex={-1}
          disabled={!canGoNext}
          aria-label={t("workflowRun.theater.parallelNext")}
          className={cn(
            "absolute right-0 top-1/2 z-20 hidden size-9 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full border border-border/80 bg-background/90 shadow-sm backdrop-blur-sm transition-[opacity,transform] duration-200 sm:flex",
            canGoNext
              ? "opacity-70 hover:scale-105 hover:opacity-100"
              : "pointer-events-none opacity-0",
            dragging && dragProgress < -0.15 && "scale-110 opacity-100",
          )}
          onClick={() => slideTo(committedIndex + 1)}
        >
          <IconChevronRight className="size-4" />
        </button>

        <div
          ref={trackRef}
          role="group"
          tabIndex={0}
          aria-roledescription="carousel"
          aria-label={t("workflowRun.theater.parallelSwitch")}
          className={cn(
            "relative touch-pan-y overflow-hidden rounded-2xl px-1 outline-none focus-visible:ring-2 focus-visible:ring-ring",
            acts.length > 1 && "cursor-grab active:cursor-grabbing",
          )}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={finishPointer}
          onPointerCancel={finishPointer}
          onKeyDown={onKeyDown}
        >
          <div
            className="pointer-events-none absolute inset-y-0 left-0 z-10 w-10 bg-gradient-to-r from-background via-background/70 to-transparent"
            aria-hidden
          />
          <div
            className="pointer-events-none absolute inset-y-0 right-0 z-10 w-10 bg-gradient-to-l from-background via-background/70 to-transparent"
            aria-hidden
          />

          <div
            className={cn(
              "flex w-full will-change-transform",
              !dragging && "motion-reduce:transition-none",
            )}
            style={{
              transform: `translateX(calc(${-index * 100}% + ${dragX}px))`,
              transition: trackTransition,
            }}
          >
            {acts.map((act, actIndex) => {
              const live = isNodeWorking(act.state.status);
              const distance = Math.abs(actIndex - index - dragProgress);
              const inactive = distance > 0.08;
              // Soft neighbor peek —avoid harsh scale jumps on programmed slides.
              const scale = inactive
                ? Math.max(0.94, 1 - distance * 0.04)
                : 1 - Math.min(0.02, Math.abs(dragProgress) * 0.02);
              const opacity = inactive
                ? Math.max(0.55, 1 - distance * 0.28)
                : 1;
              const tilt = actIndex === index ? dragProgress * -3 : 0;

              return (
                <div
                  key={act.nodeId}
                  className="w-full shrink-0 px-2"
                  aria-hidden={act.nodeId !== primaryId}
                >
                  <div
                    className="origin-center will-change-transform"
                    style={{
                      transform: `scale(${scale}) rotate(${tilt}deg)`,
                      opacity,
                      transition: dragging
                        ? "none"
                        : `transform ${SLIDE_MS}ms ${SNAP_EASE}, opacity 360ms ease-out`,
                    }}
                  >
                    <RunTheaterActCard
                      data={act.data}
                      state={act.state}
                      runId={runId}
                      nodeId={act.nodeId}
                      live={live}
                      artifactCount={act.artifactCount}
                      conversation={act.conversation}
                      conversationEnabled={
                        act.nodeId === primaryId && act.data.kind === "agent"
                      }
                      conversationOpen={
                        sessionConversationNodeId === act.nodeId
                      }
                      onConversationOpenChange={
                        act.nodeId === primaryId && act.data.kind === "agent"
                          ? (open) => {
                              onSessionConversationNodeIdChange?.(
                                open ? act.nodeId : null,
                              );
                            }
                          : undefined
                      }
                      onNodeCompleted={onNodeCompleted}
                      variant="stage"
                      emphasized={act.nodeId === primaryId}
                      inspectorOpen={inspectorOpen}
                      onToggleInspector={
                        act.nodeId === primaryId ? onToggleInspector : undefined
                      }
                      interaction={
                        act.nodeId === primaryId
                          ? primaryInteraction
                          : undefined
                      }
                    />
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      <div className="mt-5 flex flex-col items-center gap-3">
        <p className="text-center text-[11px] text-muted-foreground">
          {t("workflowRun.theater.parallelHint", {
            count: acts.length,
            index: committedIndex + 1,
          })}
        </p>

        <div className="flex items-center gap-1.5" aria-hidden>
          {acts.map((act, actIndex) => (
            <span
              key={act.nodeId}
              className={cn(
                "h-1.5 rounded-full transition-[width,background-color,transform] duration-300 ease-out",
                actIndex === committedIndex
                  ? "w-5 scale-100 bg-foreground"
                  : "w-1.5 bg-muted-foreground/35",
              )}
            />
          ))}
        </div>

        <div className="flex min-w-0 flex-wrap justify-center gap-1.5">
          {acts.map((act, actIndex) => {
            const selected = act.nodeId === primaryId;
            const waiting = act.state.status === "awaiting_input";
            return (
              <button
                key={act.nodeId}
                type="button"
                onClick={() => slideTo(actIndex)}
                className={cn(
                  "max-w-[9rem] cursor-pointer truncate rounded-full border px-2.5 py-1 font-sans text-[11px] font-medium transition-[colors,box-shadow] duration-200",
                  selected && waiting
                    ? "border-amber-500/55 bg-amber-500/15 text-amber-950 shadow-sm dark:text-amber-50"
                    : selected
                      ? "border-foreground/35 bg-background shadow-sm"
                      : waiting
                        ? "border-amber-500/40 bg-amber-500/10 text-amber-950 dark:text-amber-100"
                        : "border-border/70 bg-muted/40 text-muted-foreground hover:border-border hover:bg-background hover:text-foreground",
                )}
                aria-pressed={selected}
                aria-label={t("workflowRun.theater.focusAct", {
                  name: act.data.title,
                })}
              >
                {act.data.title}
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}
