import { diffLineScrollTop } from "./task-diff-scroll";

/**
 * Frames a jump keeps polling for the cited row before giving up. Bounded so a
 * row that never mounts (e.g. its block stayed collapsed) does not scroll forever.
 */
export const MAX_JUMP_ALIGN_ATTEMPTS = 30;

/**
 * Frames to re-derive the centered offset after the first scroll. A windowed
 * chunk re-measures (via `measureElement`) a frame or two after it mounts, so a
 * single scroll may leave the row slightly off-center; re-checking for a couple
 * more frames lets that settle without polling until the retry cap.
 */
const SETTLE_FRAMES = 2;

export interface DiffLineAligner {
  /** Starts (or restarts) polling until the row mounts and is centered. */
  align: () => void;
  /** Stops any in-flight poll; safe to call repeatedly, and from unmount. */
  cancel: () => void;
}

export interface DiffLineAlignerOptions {
  /** Pane (`.ora-diff-scroll-region`) that custom scrolls. */
  getRegion: () => HTMLElement | null;
  /** Returns the currently-highlighted row, or null when it is not mounted. */
  getRow: () => HTMLElement | null;
  /** Timers are injectable so the module stays unit-testable in jsdom. */
  requestFrame?: (cb: () => void) => number;
  cancelFrame?: (handle: number) => void;
}

/**
 * A cancellable, re-entrant scroll aligner for a chat-citation jump.
 *
 * It repeatedly looks for the cited row (which in windowed mode is not mounted
 * until the virtualizer scrolls to its chunk) and, once found, vertically
 * centers it inside the pane. It is deliberately framework-free: the caller
 * owns the React lifecycle and just calls `align()` to (re)start and `cancel()`
 * to stop, so a new jump replaces the previous poll instead of competing with a
 * stale one — the property that keeps repeated jumps in one pane reliable.
 */
export function createDiffLineAligner(
  options: DiffLineAlignerOptions,
): DiffLineAligner {
  const {
    getRegion,
    getRow,
    requestFrame = (cb) => requestAnimationFrame(cb),
    cancelFrame = (handle) => cancelAnimationFrame(handle),
  } = options;
  let frame = 0;
  let attempts = 0;
  let settle = 0;

  const tick = () => {
    frame = 0;
    const region = getRegion();
    const row = getRow();
    if (region === null || row === null) {
      // Row not mounted yet (windowed chunk loading): keep polling.
      if (attempts < MAX_JUMP_ALIGN_ATTEMPTS) {
        attempts += 1;
        frame = requestFrame(tick);
      }
      return;
    }
    if (typeof region.scrollTo !== "function") return;
    const top = diffLineScrollTop(region, row);
    if (top === null) {
      if (attempts < MAX_JUMP_ALIGN_ATTEMPTS) {
        attempts += 1;
        frame = requestFrame(tick);
      }
      return;
    }
    // Scroll only vertically (block: center) while persisting scrollLeft, so a
    // jump to a long line never yanks the whole diff sideways.
    region.scrollTo({ top, left: region.scrollLeft });
    // Re-check for a couple frames so `measureElement` settling does not leave
    // the row off-center; the recursion below re-derives the offset each frame.
    if (settle < SETTLE_FRAMES) {
      settle += 1;
      frame = requestFrame(tick);
    } else {
      attempts = 0;
      settle = 0;
    }
  };

  return {
    align() {
      // Cancel any in-flight poll first so a new jump replaces the old one.
      if (frame !== 0) cancelFrame(frame);
      attempts = 0;
      settle = 0;
      tick();
    },
    cancel() {
      if (frame !== 0) cancelFrame(frame);
      frame = 0;
      attempts = 0;
      settle = 0;
    },
  };
}
