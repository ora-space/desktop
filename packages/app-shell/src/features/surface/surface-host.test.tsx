import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { PlatformProvider } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { createSurfaceTestPlatform } from "../../test/surface-test-platform";
import { SurfaceHost } from "./surface-host";

let resizeCallback: ResizeObserverCallback | undefined;
let frames: FrameRequestCallback[] = [];
const originalResizeObserver = globalThis.ResizeObserver;
const originalRaf = window.requestAnimationFrame;
const originalCancelRaf = window.cancelAnimationFrame;

/** Runs every queued animation frame callback once, like a single paint. */
function flushFrames() {
  const pending = frames;
  frames = [];
  for (const frame of pending) frame(performance.now());
}

beforeEach(() => {
  globalThis.ResizeObserver = class {
    constructor(private readonly callback: ResizeObserverCallback) {
      resizeCallback = callback;
    }
    observe() {}
    unobserve() {}
    // A disconnected observer never fires again, so drop the test handle too.
    disconnect() {
      if (resizeCallback === this.callback) resizeCallback = undefined;
    }
  } as unknown as typeof ResizeObserver;
  window.requestAnimationFrame = (callback) => {
    frames.push(callback);
    return frames.length;
  };
  window.cancelAnimationFrame = vi.fn();
  useSurfaceStore.setState({
    embeddedSupported: true,
    records: {},
    failures: {},
    sidePanelInstance: 1,
  });
});

afterEach(() => {
  globalThis.ResizeObserver = originalResizeObserver;
  window.requestAnimationFrame = originalRaf;
  window.cancelAnimationFrame = originalCancelRaf;
  frames = [];
});

function renderHost() {
  const host = createSurfaceTestPlatform({ embedded: true });
  render(
    <AppI18nProvider>
      <PlatformProvider adapter={host.platform}>
        <SurfaceHost instance={1} />
      </PlatformProvider>
    </AppI18nProvider>,
  );
  return host;
}

describe("SurfaceHost bounds sync", () => {
  it("sends bounds once on show and coalesces resize bursts per animation frame", () => {
    const host = renderHost();
    expect(host.surfaces.setBounds).toHaveBeenCalledTimes(1);
    expect(host.surfaces.setBounds).toHaveBeenCalledWith(1, {
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      scale: window.devicePixelRatio,
    });

    const observer = {} as ResizeObserver;
    resizeCallback?.([], observer);
    resizeCallback?.([], observer);
    resizeCallback?.([], observer);
    expect(host.surfaces.setBounds).toHaveBeenCalledTimes(1);
    expect(frames).toHaveLength(1);

    flushFrames();
    expect(host.surfaces.setBounds).toHaveBeenCalledTimes(2);
  });

  it("stops sending while hidden and resumes with a fresh measurement", () => {
    const host = renderHost();
    host.surfaces.setBounds.mockClear();

    // Another instance taking the slot hides this one.
    act(() => useSurfaceStore.getState().setSidePanelInstance(2));
    expect(host.surfaces.setVisible).toHaveBeenLastCalledWith(1, false);
    resizeCallback?.([], {} as ResizeObserver);
    flushFrames();
    expect(host.surfaces.setBounds).not.toHaveBeenCalled();

    act(() => useSurfaceStore.getState().setSidePanelInstance(1));
    expect(host.surfaces.setVisible).toHaveBeenLastCalledWith(1, true);
    expect(host.surfaces.setBounds).toHaveBeenCalledTimes(1);
  });

  it("hides the previous instance when the layout replaces the keyed host", () => {
    const host = createSurfaceTestPlatform({ embedded: true });
    // Mirrors `WorkspaceReviewLayout`, which keys the host by the slot instance so a
    // switch unmounts the old host instead of re-rendering it hidden.
    function Slot({ instance }: { instance: number }) {
      return (
        <AppI18nProvider>
          <PlatformProvider adapter={host.platform}>
            <SurfaceHost key={instance} instance={instance} />
          </PlatformProvider>
        </AppI18nProvider>
      );
    }
    const view = render(<Slot instance={1} />);
    host.surfaces.setVisible.mockClear();

    act(() => {
      useSurfaceStore.getState().setSidePanelInstance(2);
      view.rerender(<Slot instance={2} />);
    });

    // The last word on each instance is what the native layer ends up showing.
    const lastVisibility = new Map<number, boolean>();
    for (const [instance, visible] of host.surfaces.setVisible.mock.calls) {
      lastVisibility.set(instance as number, visible as boolean);
    }
    expect([...lastVisibility.entries()]).toEqual([
      [1, false],
      [2, true],
    ]);
  });
});
