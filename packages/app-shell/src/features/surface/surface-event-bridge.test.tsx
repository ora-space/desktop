import { act, render } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { PlatformProvider, type SurfaceRecord } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { createSurfaceTestPlatform } from "../../test/surface-test-platform";
import { SurfaceEventBridge } from "./surface-event-bridge";

function record(
  instance: number,
  target: "embedded" | "windowed",
): SurfaceRecord {
  return {
    instance,
    pluginId: "acme.hub",
    kind: "webview" as const,
    title: "Example Hub",
    target,
    state: "open",
  };
}

beforeEach(() => {
  useSurfaceStore.setState({
    embeddedSupported: null,
    records: {},
    failures: {},
    sidePanelInstance: null,
  });
});

describe("SurfaceEventBridge hydrate", () => {
  it("hides surviving embedded instances and keeps events that raced the snapshot", async () => {
    const host = createSurfaceTestPlatform({ embedded: true });
    let resolveList: ((records: SurfaceRecord[]) => void) | undefined;
    host.surfaces.list.mockImplementation(
      () =>
        new Promise<SurfaceRecord[]>((resolve) => {
          resolveList = resolve;
        }),
    );

    render(
      <PlatformProvider adapter={host.platform}>
        <SurfaceEventBridge />
      </PlatformProvider>,
    );
    // The subscription is established before the snapshot is requested.
    await act(async () => {
      await Promise.resolve();
    });
    expect(host.surfaces.onEvent).toHaveBeenCalledTimes(1);

    // Instance 2 closes while the snapshot (which still lists it) is in flight.
    act(() => host.emit({ type: "closed", instance: 2 }));
    await act(async () => {
      resolveList?.([
        record(1, "embedded"),
        record(2, "windowed"),
        record(3, "windowed"),
      ]);
      await Promise.resolve();
    });

    expect({
      instances: Object.keys(useSurfaceStore.getState().records),
      sidePanelInstance: useSurfaceStore.getState().sidePanelInstance,
      hidden: host.surfaces.setVisible.mock.calls,
    }).toEqual({
      instances: ["1", "3"],
      sidePanelInstance: null,
      hidden: [[1, false]],
    });
  });
});
