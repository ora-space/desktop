import { describe, expect, it, vi } from "vitest";
import {
  createDiffLineAligner,
  MAX_JUMP_ALIGN_ATTEMPTS,
} from "./task-diff-jump";

/** A synchronous frame scheduler so tests step the poll deterministically. */
function manualScheduler() {
  const queue: Array<() => void> = [];
  return {
    requestFrame: (cb: () => void) => {
      queue.push(cb);
      return queue.length;
    },
    cancelFrame: (handle: number) => {
      // Marks the slot as cancelled by swapping in a no-op; `flush` still runs,
      // so a cancelled tick's effect (scroll) is observable as a no-scroll.
      if (handle > 0 && queue[handle - 1] !== undefined) {
        queue[handle - 1] = () => undefined;
      }
    },
    flush: (count: number) => {
      for (let i = 0; i < count; i += 1) {
        const cb = queue.shift();
        if (cb !== undefined) cb();
      }
    },
  };
}

function makeRow(top: number) {
  const el = document.createElement("div");
  el.className = "diff-code-selected";
  Object.defineProperty(el, "offsetHeight", { value: 20 });
  Object.defineProperty(el, "getBoundingClientRect", {
    value: () => ({
      top,
      bottom: top + 20,
      left: 0,
      right: 0,
      width: 0,
      height: 20,
      toJSON: () => ({}),
    }),
  });
  return el;
}

function makeRegion(height: number, scrollTop = 0) {
  const el = document.createElement("div");
  Object.defineProperty(el, "clientHeight", { value: height });
  Object.defineProperty(el, "offsetHeight", { value: height });
  Object.defineProperty(el, "scrollTop", {
    value: scrollTop,
    writable: true,
    configurable: true,
  });
  Object.defineProperty(el, "scrollLeft", { value: 0, writable: true });
  const scrollTo = vi.fn();
  Object.defineProperty(el, "scrollTo", {
    value: (opts: { top: number; left: number }) =>
      scrollTo({ top: opts.top, left: opts.left }) as unknown,
  });
  Object.defineProperty(el, "getBoundingClientRect", {
    value: () => ({ top: 0, bottom: height, left: 0, right: 0, height }),
  });
  (el as unknown as { __scrollTo: ReturnType<typeof vi.fn> }).__scrollTo =
    scrollTo;
  return { el, scrollTo };
}

describe("createDiffLineAligner", () => {
  it("polls until the row mounts, then centers it", () => {
    const s = manualScheduler();
    let mounted = false;
    const region = makeRegion(400);
    const aligner = createDiffLineAligner({
      getRegion: () => region.el,
      getRow: () => (mounted ? makeRow(1000) : null),
      requestFrame: s.requestFrame,
      cancelFrame: s.cancelFrame,
    });
    aligner.align();
    // Row not mounted yet: keep polling, no scroll.
    s.flush(1);
    expect(region.scrollTo).not.toHaveBeenCalled();
    mounted = true;
    s.flush(1);
    // 1000 content offset; center pulls up by (400-20)/2 = 190 → top 810.
    s.flush(2); // settle frames
    expect(region.scrollTo).toHaveBeenCalledWith({ top: 810, left: 0 });
  });

  it("a new align replaces the previous poll (re-entrant)", () => {
    const s = manualScheduler();
    let row: HTMLElement | null = makeRow(1000);
    const region = makeRegion(400);
    const aligner = createDiffLineAligner({
      getRegion: () => region.el,
      getRow: () => row,
      requestFrame: s.requestFrame,
      cancelFrame: s.cancelFrame,
    });
    aligner.align();
    s.flush(1);
    // Start a second align with a different row; the first poll is cancelled
    // so only the new target is scrolled to.
    row = makeRow(500);
    aligner.align();
    s.flush(1);
    s.flush(2);
    expect(region.scrollTo).toHaveBeenLastCalledWith({ top: 310, left: 0 });
  });

  it("cancel stops any pending poll", () => {
    const s = manualScheduler();
    const region = makeRegion(400);
    const aligner = createDiffLineAligner({
      getRegion: () => region.el,
      getRow: () => null,
      requestFrame: s.requestFrame,
      cancelFrame: s.cancelFrame,
    });
    aligner.align();
    aligner.cancel();
    const countBefore = region.scrollTo.mock.calls.length;
    s.flush(MAX_JUMP_ALIGN_ATTEMPTS);
    // Cancelled poll never scrolls.
    expect(region.scrollTo.mock.calls.length).toBe(countBefore);
  });
});
