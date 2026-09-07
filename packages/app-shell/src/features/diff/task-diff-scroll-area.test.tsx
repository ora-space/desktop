import { createRef } from "react";
import { act, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TaskDiffScrollArea } from "./task-diff-scroll-area";
import {
  diffScrollbarMetrics,
  diffScrollTopForDrag,
  usesWindowsDiffScrollbar,
} from "./task-diff-scrollbar";

const nextAnimationFrame = async () => {
  await act(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => resolve());
      }),
  );
};

/** Dispatches pointer coordinates that jsdom's generic Event omits. */
function pointerEvent(
  target: Element,
  type: string,
  values: {
    pointerId: number;
    clientX: number;
    clientY: number;
    button?: number;
  },
) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  for (const [key, value] of Object.entries(values)) {
    Object.defineProperty(event, key, { configurable: true, value });
  }
  fireEvent(target, event);
}

afterEach(() => vi.restoreAllMocks());

describe("diff scrollbar geometry", () => {
  it("computes one complete thumb geometry object", () => {
    expect(
      diffScrollbarMetrics({
        viewportHeight: 400,
        scrollHeight: 2_000,
        trackHeight: 200,
        scrollTop: 800,
      }),
    ).toEqual({
      hidden: false,
      maxScrollTop: 1_600,
      thumbHeight: 40,
      thumbOffset: 80,
      thumbTravel: 160,
    });
  });

  it("maps drag distance through the frozen drag-start range", () => {
    expect(
      diffScrollTopForDrag(
        {
          startScrollTop: 1_000,
          maxScrollTop: 9_600,
          thumbTravel: 320,
        },
        160,
      ),
    ).toBe(5_800);
  });

  it("detects only Windows user agents", () => {
    expect({
      windows: usesWindowsDiffScrollbar(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/140.0.0.0 Edg/140.0.0.0",
      ),
      windowsFirefox: usesWindowsDiffScrollbar(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Firefox/142.0",
      ),
      macos: usesWindowsDiffScrollbar(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X) Chrome/140.0.0.0",
      ),
      linux: usesWindowsDiffScrollbar("Mozilla/5.0 (X11; Linux x86_64)"),
    }).toEqual({
      windows: true,
      windowsFirefox: false,
      macos: false,
      linux: false,
    });
  });
});

describe("TaskDiffScrollArea", () => {
  it("keeps the native scroll region on non-Windows hosts", () => {
    vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue(
      "Mozilla/5.0 (X11; Linux x86_64)",
    );
    const viewportRef = createRef<HTMLDivElement>();
    const { container } = render(
      <TaskDiffScrollArea viewportRef={viewportRef}>
        <div>diff</div>
      </TaskDiffScrollArea>,
    );

    expect(container.querySelector(".ora-diff-scroll-shell")).toBeNull();
    expect(container.querySelector("[data-diff-scrollbar]")).toBeNull();
    expect(viewportRef.current).toHaveClass("ora-diff-scroll-region");
  });

  it("coalesces a Windows drag, ignores X, and freezes its scroll range", async () => {
    vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue(
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/140.0.0.0 Edg/140.0.0.0",
    );
    const viewportRef = createRef<HTMLDivElement>();
    const { container } = render(
      <TaskDiffScrollArea viewportRef={viewportRef}>
        <div style={{ height: 10_000 }}>diff</div>
      </TaskDiffScrollArea>,
    );
    const viewport = viewportRef.current!;
    const track = container.querySelector<HTMLElement>(
      '[data-diff-scrollbar="vertical"]',
    )!;
    const thumb = track.querySelector<HTMLElement>(
      "[data-diff-scrollbar-thumb]",
    )!;

    let scrollHeight = 10_000;
    let scrollTop = 1_000;
    let scrollWrites = 0;
    Object.defineProperties(viewport, {
      clientHeight: { configurable: true, get: () => 400 },
      scrollHeight: { configurable: true, get: () => scrollHeight },
      scrollTop: {
        configurable: true,
        get: () => scrollTop,
        set: (value: number) => {
          scrollTop = value;
          scrollWrites += 1;
        },
      },
    });
    Object.defineProperties(track, {
      clientHeight: { configurable: true, get: () => 348 },
      getBoundingClientRect: {
        configurable: true,
        value: () => ({
          x: 0,
          y: 40,
          top: 40,
          right: 14,
          bottom: 388,
          left: 0,
          width: 14,
          height: 348,
          toJSON: () => ({}),
        }),
      },
    });
    const capturedPointers = new Set<number>();
    Object.defineProperties(track, {
      setPointerCapture: {
        configurable: true,
        value: vi.fn((pointerId: number) => capturedPointers.add(pointerId)),
      },
      hasPointerCapture: {
        configurable: true,
        value: (pointerId: number) => capturedPointers.has(pointerId),
      },
      releasePointerCapture: {
        configurable: true,
        value: vi.fn((pointerId: number) => capturedPointers.delete(pointerId)),
      },
    });

    fireEvent.scroll(viewport);
    await nextAnimationFrame();
    expect(track).toHaveAttribute("data-scrollbar-hidden", "false");
    expect(track).toHaveAttribute("aria-hidden", "false");
    expect(track).toHaveAttribute("tabindex", "0");
    expect(track.parentElement).toBe(viewport.parentElement);

    pointerEvent(thumb, "pointerdown", {
      pointerId: 7,
      button: 0,
      clientX: 10,
      clientY: 100,
    });
    expect(track.setPointerCapture).toHaveBeenCalledWith(7);
    scrollHeight = 20_000;
    scrollWrites = 0;

    for (let index = 0; index < 20; index += 1) {
      pointerEvent(track, "pointermove", {
        pointerId: 7,
        clientX: -1_000,
        clientY: 101 + index * 5,
      });
    }
    expect(scrollWrites).toBe(0);

    await nextAnimationFrame();
    expect(scrollWrites).toBe(1);
    expect(scrollTop).toBeGreaterThan(1_000);
    // The new 20k content height is ignored until release: this value uses the
    // 10k drag-start range and therefore cannot jump with virtual measurements.
    expect(scrollTop).toBeLessThan(5_000);

    pointerEvent(track, "pointerup", {
      pointerId: 7,
      clientX: -1_000,
      clientY: 196,
    });
    await nextAnimationFrame();
    expect(track.releasePointerCapture).toHaveBeenCalledWith(7);
    expect(track).toHaveAttribute("aria-valuemax", "19600");
    expect(scrollTop).toBeGreaterThan(1_000);
  });

  it("supports keyboard and wheel scrolling on the Windows rail", async () => {
    vi.spyOn(window.navigator, "userAgent", "get").mockReturnValue(
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/140.0.0.0 Edg/140.0.0.0",
    );
    const viewportRef = createRef<HTMLDivElement>();
    const { container } = render(
      <TaskDiffScrollArea viewportRef={viewportRef}>
        <div>diff</div>
      </TaskDiffScrollArea>,
    );
    const viewport = viewportRef.current!;
    const track = container.querySelector<HTMLElement>(
      '[data-diff-scrollbar="vertical"]',
    )!;
    let scrollTop = 0;
    Object.defineProperties(viewport, {
      clientHeight: { configurable: true, get: () => 400 },
      scrollHeight: { configurable: true, get: () => 2_000 },
      scrollTop: {
        configurable: true,
        get: () => scrollTop,
        set: (value: number) => {
          scrollTop = value;
        },
      },
    });
    Object.defineProperty(track, "clientHeight", {
      configurable: true,
      get: () => 350,
    });
    fireEvent.scroll(viewport);
    await nextAnimationFrame();

    fireEvent.keyDown(track, { key: "PageDown" });
    expect(scrollTop).toBe(400);
    fireEvent.wheel(track, { deltaY: 75, deltaMode: 0 });
    expect(scrollTop).toBe(475);
    fireEvent.keyDown(track, { key: "End" });
    expect(scrollTop).toBe(1_600);
  });
});
