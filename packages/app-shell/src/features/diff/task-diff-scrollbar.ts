const MIN_THUMB_HEIGHT_PX = 28;

export interface DiffScrollbarMetrics {
  hidden: boolean;
  maxScrollTop: number;
  thumbHeight: number;
  thumbOffset: number;
  thumbTravel: number;
}

interface DiffScrollbarGeometry {
  viewportHeight: number;
  scrollHeight: number;
  trackHeight: number;
  scrollTop: number;
}

interface DiffScrollbarDragRange {
  startScrollTop: number;
  maxScrollTop: number;
  thumbTravel: number;
}

/** Keeps a number inside an inclusive range without propagating invalid geometry. */
function clamp(value: number, minimum: number, maximum: number): number {
  if (!Number.isFinite(value)) return minimum;
  return Math.min(maximum, Math.max(minimum, value));
}

/** Computes the vertical thumb geometry from one stable viewport measurement. */
export function diffScrollbarMetrics({
  viewportHeight,
  scrollHeight,
  trackHeight,
  scrollTop,
}: DiffScrollbarGeometry): DiffScrollbarMetrics {
  const viewport = Math.max(0, viewportHeight);
  const content = Math.max(0, scrollHeight);
  const track = Math.max(0, trackHeight);
  const maxScrollTop = Math.max(0, content - viewport);
  if (viewport === 0 || track === 0 || maxScrollTop === 0) {
    return {
      hidden: true,
      maxScrollTop,
      thumbHeight: track,
      thumbOffset: 0,
      thumbTravel: 0,
    };
  }

  const thumbHeight = Math.min(
    track,
    Math.max(MIN_THUMB_HEIGHT_PX, (viewport / content) * track),
  );
  const thumbTravel = Math.max(0, track - thumbHeight);
  const normalizedScrollTop = clamp(scrollTop, 0, maxScrollTop);

  return {
    hidden: false,
    maxScrollTop,
    thumbHeight,
    thumbOffset:
      thumbTravel === 0
        ? 0
        : (normalizedScrollTop / maxScrollTop) * thumbTravel,
    thumbTravel,
  };
}

/** Maps a pointer delta through the drag-start snapshot, independent of X. */
export function diffScrollTopForDrag(
  drag: DiffScrollbarDragRange,
  pointerDeltaY: number,
): number {
  if (drag.thumbTravel <= 0 || drag.maxScrollTop <= 0) return 0;
  return clamp(
    drag.startScrollTop +
      (pointerDeltaY / drag.thumbTravel) * drag.maxScrollTop,
    0,
    drag.maxScrollTop,
  );
}

/** Limits the custom control to Windows Chromium/WebView2, where snap-back applies. */
export function usesWindowsDiffScrollbar(userAgent: string): boolean {
  return (
    /Windows|Win32|Win64|WOW64/iu.test(userAgent) &&
    /(?:Chrome|Chromium|Edg|WebView)\/[\d.]+/iu.test(userAgent)
  );
}
