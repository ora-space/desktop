import { act, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import "../../i18n/i18n-instance";
import {
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import { usePluginOperationStore } from "../stores/plugin-operation-store";
import { useUpdatePlugin } from "./use-update-plugin";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return { ...createPluginMemory() };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...pluginHandlers(state),
  };
}

afterEach(() => {
  act(() => usePluginOperationStore.setState({ activities: {} }));
});

describe("useUpdatePlugin", () => {
  it("updates an installed plugin and refreshes the installed surface", async () => {
    const state = createFixtureState();
    state.availablePlugins.push({
      id: "official/weather",
      name: "weather",
      title: "Weather",
      kind: "agent",
      namespace: "official",
      sourceUrl: "https://github.com/ora-space/marketplace",
      version: "1.1.0",
      description: "Weather",
      logo: null,
      compatibility: "compatible",
    });
    state.installedPlugins.push({
      id: "official/weather",
      namespace: "official",
      name: "weather",
      displayName: "weather",
      version: "1.0.0",
      description: "Weather",
      homepage: null,
      license: null,
      kind: "agent",
      agentDisplayName: "weather",
      logo: null,
      installationValidity: { validity: "valid" },
      configuration: { state: "not_declared" },
      runtime: "stopped",
    });
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const { result } = renderHookWithClient(
      () => useUpdatePlugin("official/weather"),
      client,
    );

    act(() => result.current.mutate({}));

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(
      state.installedPlugins.find((item) => item.id === "official/weather"),
    ).toMatchObject({
      id: "official/weather",
      version: "1.1.0",
    });
  });

  it("keeps update progress across unmount and rejects a duplicate start", async () => {
    const state = createFixtureState();
    const handlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(handlers);
    let resolveUpdate: ((response: { pluginId: string }) => void) | undefined;
    // The update stays in flight so the durable operation state, not the query cache, is what
    // carries progress across the unmount below.
    const update = vi.spyOn(handlers, "updatePlugin").mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveUpdate = resolve;
        }),
    );
    const queryClient = createTestQueryClient();
    const first = renderHookWithClient(
      () => useUpdatePlugin("official/weather"),
      client,
      queryClient,
    );

    act(() => first.result.current.mutate({}));
    await waitFor(() => expect(update).toHaveBeenCalledOnce());
    act(() => {
      usePluginOperationStore.getState().reportTransferProgress({
        pluginId: "official/weather",
        downloaded: 4,
        total: 10,
      });
    });
    expect(first.result.current.progress).toEqual({
      pluginId: "official/weather",
      downloaded: 4,
      total: 10,
    });
    first.unmount();

    const second = renderHookWithClient(
      () => useUpdatePlugin("official/weather"),
      client,
      queryClient,
    );
    expect(second.result.current.isPending).toBe(true);
    expect(second.result.current.progress?.downloaded).toBe(4);
    act(() => second.result.current.mutate({}));
    expect(update).toHaveBeenCalledOnce();

    await act(async () => {
      resolveUpdate?.({ pluginId: "official/weather" });
    });
    await waitFor(() => expect(second.result.current.isPending).toBe(false));
  });
});
