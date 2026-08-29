import { beforeEach, describe, expect, it } from "vitest";
import { useSurfaceStore } from "./surface-store";

beforeEach(() => {
  useSurfaceStore.setState({
    embeddedSupported: null,
    records: {},
    failures: {},
    sidePanelInstance: null,
    sidePanelClaimTick: 0,
  });
});

describe("surface store failures", () => {
  it("clears the failure reason when a rebuilt instance opens again", () => {
    const store = useSurfaceStore.getState();
    store.applyEvent({
      type: "opened",
      instance: 4,
      pluginId: "acme.hub",
      kind: "webview" as const,
      target: "embedded",
      title: "Example Hub",
    });
    store.applyEvent({ type: "failed", instance: 4, reason: "boom" });
    const failed = {
      state: useSurfaceStore.getState().records[4]?.state,
      failure: useSurfaceStore.getState().failures[4],
    };

    // A retry rebuilds the same instance id, so `opened` arrives for it again.
    store.applyEvent({
      type: "opened",
      instance: 4,
      pluginId: "acme.hub",
      kind: "webview" as const,
      target: "embedded",
      title: "Example Hub",
    });

    expect({
      failed,
      recovered: {
        state: useSurfaceStore.getState().records[4]?.state,
        failures: useSurfaceStore.getState().failures,
      },
    }).toEqual({
      failed: { state: "failed", failure: "boom" },
      recovered: { state: "open", failures: {} },
    });
  });
});

describe("surface store side-slot claims", () => {
  it("bumps the claim tick on claims and keeps it across releases", () => {
    useSurfaceStore.getState().setSidePanelInstance(3);
    expect(useSurfaceStore.getState().sidePanelClaimTick).toBe(1);

    useSurfaceStore.getState().setSidePanelInstance(null);
    expect(useSurfaceStore.getState().sidePanelClaimTick).toBe(1);

    // Re-claiming the instance already in the slot must stay observable so the
    // review layout can win the panel back after another panel took it over.
    useSurfaceStore.getState().setSidePanelInstance(3);
    expect(useSurfaceStore.getState().sidePanelClaimTick).toBe(2);
  });
});
