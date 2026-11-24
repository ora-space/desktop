import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import {
  PlatformProvider,
  type DesktopUpdateCapability,
  type DesktopUpdateStatus,
} from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import { useDesktopUpdate } from "./use-desktop-update";

const READY: DesktopUpdateStatus = {
  kind: "ready",
  version: "0.3.0",
  notes: [],
};

/** Follows an updater whose snapshot the test answers by hand. */
function renderFollower({
  duringSubscribe,
}: { duringSubscribe?: DesktopUpdateStatus } = {}) {
  let resolve: (status: DesktopUpdateStatus) => void = () => undefined;
  const answer = new Promise<DesktopUpdateStatus>((settle) => {
    resolve = settle;
  });
  const snapshot = { promise: answer, resolve };
  const getStatus = vi.fn(() => answer);
  const stop = vi.fn();
  let listener: ((next: DesktopUpdateStatus) => void) | undefined;
  const updates: DesktopUpdateCapability = {
    getStatus,
    install: async () => undefined,
    check: async () => undefined,
    onStatus: async (next) => {
      listener = next;
      if (duringSubscribe !== undefined) next(duringSubscribe);
      return stop;
    },
  };
  const platform = { ...createStubPlatform(), updates };
  const hook = renderHook(() => useDesktopUpdate(), {
    wrapper: ({ children }: { children: ReactNode }) => (
      <PlatformProvider adapter={platform}>{children}</PlatformProvider>
    ),
  });
  return {
    ...hook,
    getStatus,
    snapshot,
    stop,
    publish: (next: DesktopUpdateStatus) => act(() => listener?.(next)),
  };
}

describe("useDesktopUpdate", () => {
  it("keeps an event over a snapshot that answers later", async () => {
    const { result, getStatus, snapshot, publish } = renderFollower();
    await waitFor(() => expect(getStatus).toHaveBeenCalled());

    publish(READY);
    await act(async () => {
      snapshot.resolve({
        kind: "downloading",
        version: "0.3.0",
        downloaded: 1,
        total: 2,
      });
      await snapshot.promise;
    });

    expect(result.current.status).toEqual(READY);
  });

  it("lets the snapshot replace an event from before it was requested", async () => {
    const { result, snapshot } = renderFollower({
      duringSubscribe: {
        kind: "downloading",
        version: "0.3.0",
        downloaded: 1,
        total: 2,
      },
    });

    await waitFor(() => expect(result.current.status.kind).toBe("downloading"));
    await act(async () => {
      snapshot.resolve(READY);
      await snapshot.promise;
    });

    await waitFor(() => expect(result.current.status).toEqual(READY));
  });

  it("starts from the snapshot when no event arrives", async () => {
    const { result, snapshot } = renderFollower();

    await act(async () => {
      snapshot.resolve(READY);
      await snapshot.promise;
    });

    await waitFor(() => expect(result.current.status).toEqual(READY));
  });

  it("stops following when unmounted", async () => {
    const { getStatus, stop, unmount } = renderFollower();
    await waitFor(() => expect(getStatus).toHaveBeenCalled());

    unmount();

    expect(stop).toHaveBeenCalledTimes(1);
  });
});
