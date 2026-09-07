import {
  useId,
  useLayoutEffect,
  useRef,
  type MouseEventHandler,
  type ReactNode,
  type RefObject,
} from "react";
import {
  diffScrollbarMetrics,
  diffScrollTopForDrag,
  usesWindowsDiffScrollbar,
  type DiffScrollbarMetrics,
} from "./task-diff-scrollbar";

const KEYBOARD_LINE_STEP_PX = 40;

function clamp(value: number, minimum: number, maximum: number): number {
  if (!Number.isFinite(value)) return minimum;
  return Math.min(maximum, Math.max(minimum, value));
}

interface DiffScrollbarDrag {
  pointerId: number;
  startPointerY: number;
  startScrollTop: number;
  startThumbOffset: number;
  maxScrollTop: number;
  thumbTravel: number;
}

export interface TaskDiffScrollAreaProps {
  viewportRef: RefObject<HTMLDivElement | null>;
  children: ReactNode;
  /** Left-clicks inside the viewport keep the existing quote-highlight behavior. */
  onMouseDown?: MouseEventHandler<HTMLDivElement>;
}

/**
 * Preserves the native Diff viewport while replacing only its Windows vertical
 * thumb. The control updates DOM geometry at most once per animation frame so
 * a large virtualized Diff never re-renders React merely because the pointer moved.
 */
export function TaskDiffScrollArea({
  viewportRef,
  children,
  onMouseDown,
}: TaskDiffScrollAreaProps) {
  const viewportId = useId();
  const contentRef = useRef<HTMLDivElement | null>(null);
  const trackRef = useRef<HTMLDivElement | null>(null);
  const thumbRef = useRef<HTMLDivElement | null>(null);
  const customVerticalScrollbar = usesWindowsDiffScrollbar(
    typeof navigator === "undefined" ? "" : navigator.userAgent,
  );

  useLayoutEffect(() => {
    if (!customVerticalScrollbar) return;

    const viewport = viewportRef.current;
    const content = contentRef.current;
    const track = trackRef.current;
    const thumb = thumbRef.current;
    if (
      viewport === null ||
      content === null ||
      track === null ||
      thumb === null
    )
      return;

    let animationFrame: number | null = null;
    let pendingPointerY: number | null = null;
    let drag: DiffScrollbarDrag | null = null;

    const readMetrics = () =>
      diffScrollbarMetrics({
        viewportHeight: viewport.clientHeight,
        scrollHeight: viewport.scrollHeight,
        trackHeight: track.clientHeight,
        scrollTop: viewport.scrollTop,
      });

    const writeAccessibleValue = (metrics: DiffScrollbarMetrics) => {
      track.setAttribute(
        "aria-valuemax",
        String(Math.round(metrics.maxScrollTop)),
      );
      track.setAttribute(
        "aria-valuenow",
        String(Math.round(clamp(viewport.scrollTop, 0, metrics.maxScrollTop))),
      );
    };

    const syncThumb = () => {
      const metrics = readMetrics();
      track.dataset.scrollbarHidden = String(metrics.hidden);
      track.setAttribute("aria-hidden", String(metrics.hidden));
      track.tabIndex = metrics.hidden ? -1 : 0;
      thumb.style.height = `${metrics.thumbHeight}px`;
      thumb.style.transform = `translate3d(0, ${metrics.thumbOffset}px, 0)`;
      writeAccessibleValue(metrics);
    };

    const applyPendingDrag = () => {
      if (drag === null || pendingPointerY === null) return;
      const pointerDeltaY = pendingPointerY - drag.startPointerY;
      const nextScrollTop = diffScrollTopForDrag(drag, pointerDeltaY);
      const nextThumbOffset = clamp(
        drag.startThumbOffset + pointerDeltaY,
        0,
        drag.thumbTravel,
      );
      pendingPointerY = null;
      viewport.scrollTop = nextScrollTop;
      thumb.style.transform = `translate3d(0, ${nextThumbOffset}px, 0)`;
      track.setAttribute("aria-valuenow", String(Math.round(nextScrollTop)));
    };

    const runAnimationFrame = () => {
      animationFrame = null;
      if (drag !== null) {
        applyPendingDrag();
      } else {
        syncThumb();
      }
    };

    const scheduleAnimationFrame = () => {
      if (animationFrame !== null) return;
      animationFrame = requestAnimationFrame(runAnimationFrame);
    };

    const beginDrag = (event: PointerEvent) => {
      if (event.button !== 0) return;
      const metrics = readMetrics();
      if (metrics.hidden) return;

      const target = event.target;
      const pressedThumb = target instanceof Node && thumb.contains(target);
      let startScrollTop = clamp(viewport.scrollTop, 0, metrics.maxScrollTop);
      let startThumbOffset = metrics.thumbOffset;

      if (!pressedThumb) {
        const trackBounds = track.getBoundingClientRect();
        startThumbOffset = clamp(
          event.clientY - trackBounds.top - metrics.thumbHeight / 2,
          0,
          metrics.thumbTravel,
        );
        startScrollTop =
          metrics.thumbTravel === 0
            ? 0
            : (startThumbOffset / metrics.thumbTravel) * metrics.maxScrollTop;
        viewport.scrollTop = startScrollTop;
        thumb.style.transform = `translate3d(0, ${startThumbOffset}px, 0)`;
      }

      drag = {
        pointerId: event.pointerId,
        startPointerY: event.clientY,
        startScrollTop,
        startThumbOffset,
        maxScrollTop: metrics.maxScrollTop,
        thumbTravel: metrics.thumbTravel,
      };
      pendingPointerY = null;
      track.dataset.dragging = "true";
      track.setPointerCapture(event.pointerId);
      track.focus({ preventScroll: true });
      event.preventDefault();
    };

    const moveDrag = (event: PointerEvent) => {
      if (drag === null || event.pointerId !== drag.pointerId) return;
      // Only Y is retained. Moving into the Diff or file tree cannot cancel or
      // reverse the scroll gesture as it does for Chromium's native scrollbar.
      pendingPointerY = event.clientY;
      scheduleAnimationFrame();
      event.preventDefault();
    };

    const finishDrag = (event: PointerEvent) => {
      if (drag === null || event.pointerId !== drag.pointerId) return;
      if (animationFrame !== null) {
        cancelAnimationFrame(animationFrame);
        animationFrame = null;
      }
      applyPendingDrag();
      drag = null;
      pendingPointerY = null;
      delete track.dataset.dragging;
      if (track.hasPointerCapture(event.pointerId)) {
        track.releasePointerCapture(event.pointerId);
      }
      scheduleAnimationFrame();
    };

    const losePointerCapture = (event: PointerEvent) => {
      if (drag === null || event.pointerId !== drag.pointerId) return;
      applyPendingDrag();
      drag = null;
      pendingPointerY = null;
      delete track.dataset.dragging;
      scheduleAnimationFrame();
    };

    const scrollFromWheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;
      const multiplier =
        event.deltaMode === WheelEvent.DOM_DELTA_LINE
          ? KEYBOARD_LINE_STEP_PX
          : event.deltaMode === WheelEvent.DOM_DELTA_PAGE
            ? viewport.clientHeight
            : 1;
      viewport.scrollTop += event.deltaY * multiplier;
      event.preventDefault();
    };

    const scrollFromKeyboard = (event: KeyboardEvent) => {
      let nextScrollTop: number;
      switch (event.key) {
        case "ArrowUp":
          nextScrollTop = viewport.scrollTop - KEYBOARD_LINE_STEP_PX;
          break;
        case "ArrowDown":
          nextScrollTop = viewport.scrollTop + KEYBOARD_LINE_STEP_PX;
          break;
        case "PageUp":
          nextScrollTop = viewport.scrollTop - viewport.clientHeight;
          break;
        case "PageDown":
          nextScrollTop = viewport.scrollTop + viewport.clientHeight;
          break;
        case "Home":
          nextScrollTop = 0;
          break;
        case "End":
          nextScrollTop = viewport.scrollHeight - viewport.clientHeight;
          break;
        default:
          return;
      }
      viewport.scrollTop = nextScrollTop;
      scheduleAnimationFrame();
      event.preventDefault();
    };

    const resizeObserver = new ResizeObserver(scheduleAnimationFrame);
    resizeObserver.observe(viewport);
    resizeObserver.observe(content);
    resizeObserver.observe(track);
    viewport.addEventListener("scroll", scheduleAnimationFrame, {
      passive: true,
    });
    track.addEventListener("pointerdown", beginDrag);
    track.addEventListener("pointermove", moveDrag);
    track.addEventListener("pointerup", finishDrag);
    track.addEventListener("pointercancel", finishDrag);
    track.addEventListener("lostpointercapture", losePointerCapture);
    track.addEventListener("wheel", scrollFromWheel, { passive: false });
    track.addEventListener("keydown", scrollFromKeyboard);
    scheduleAnimationFrame();

    return () => {
      resizeObserver.disconnect();
      viewport.removeEventListener("scroll", scheduleAnimationFrame);
      track.removeEventListener("pointerdown", beginDrag);
      track.removeEventListener("pointermove", moveDrag);
      track.removeEventListener("pointerup", finishDrag);
      track.removeEventListener("pointercancel", finishDrag);
      track.removeEventListener("lostpointercapture", losePointerCapture);
      track.removeEventListener("wheel", scrollFromWheel);
      track.removeEventListener("keydown", scrollFromKeyboard);
      if (drag !== null && track.hasPointerCapture(drag.pointerId)) {
        track.releasePointerCapture(drag.pointerId);
      }
      if (animationFrame !== null) cancelAnimationFrame(animationFrame);
    };
  }, [customVerticalScrollbar, viewportRef]);

  const viewport = (
    <div
      id={viewportId}
      ref={viewportRef}
      className={`ora-scroll-region ora-diff-scroll-region h-full min-w-0 overflow-auto bg-background${
        customVerticalScrollbar
          ? " ora-diff-scroll-region--custom-vertical"
          : ""
      }`}
      onMouseDown={onMouseDown}
    >
      {customVerticalScrollbar ? (
        <div ref={contentRef} className="ora-diff-scroll-content">
          {children}
        </div>
      ) : (
        children
      )}
    </div>
  );

  if (!customVerticalScrollbar) return viewport;

  return (
    <div className="ora-diff-scroll-shell h-full min-w-0 bg-background">
      {viewport}
      <div
        ref={trackRef}
        data-diff-scrollbar="vertical"
        role="scrollbar"
        aria-label="Diff"
        aria-controls={viewportId}
        aria-orientation="vertical"
        aria-valuemin={0}
        aria-valuemax={0}
        aria-valuenow={0}
        aria-hidden="true"
        data-scrollbar-hidden="true"
        tabIndex={-1}
      >
        <div ref={thumbRef} data-diff-scrollbar-thumb aria-hidden="true" />
      </div>
    </div>
  );
}
