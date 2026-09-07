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
import { useInstallPlugin } from "./use-install-plugin";

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

describe("useInstallPlugin", () => {
  it("installs a marketplace plugin and refreshes the installed surface", async () => {
    const state = createFixtureState();
    state.availablePlugins.push({
      id: "official/weather",
      name: "weather",
      title: "Weather",
      kind: "agent",
      namespace: "official",
      sourceUrl: "https://github.com/ora-space/marketplace",
      version: "1.2.0",
      description: "Weather",
      logo: null,
      compatibility: "compatible",
    });
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const { result } = renderHookWithClient(
      () => useInstallPlugin("official/weather"),
      client,
    );

    act(() => result.current.mutate({}));

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(
      state.installedPlugins.find((item) => item.id === "official/weather"),
    ).toMatchObject({
      id: "official/weather",
      namespace: "official",
      name: "weather",
      displayName: "weather",
      version: "1.2.0",
    });
  });

  it("keeps an install pending across unmount and rejects a duplicate start", async () => {
    const clientHandlers: TestHandlers =
      createFixtureHandlers(createFixtureState());
    const client = createTestClient(clientHandlers);
    let resolveInstall:
      | ((response: Awaited<ReturnType<typeof client.plugin.install>>) => void)
      | undefined;
    const install = vi
      .spyOn(clientHandlers, "installPlugin")
      .mockImplementation(
        () =>
          new Promise((resolve) => {
            resolveInstall = resolve;
          }),
      );
    const queryClient = createTestQueryClient();
    const first = renderHookWithClient(
      () => useInstallPlugin("official/weather"),
      client,
      queryClient,
    );

    act(() => first.result.current.mutate({}));
    await waitFor(() => expect(install).toHaveBeenCalledOnce());
    expect(first.result.current.isPending).toBe(true);
    first.unmount();

    const second = renderHookWithClient(
      () => useInstallPlugin("official/weather"),
      client,
      queryClient,
    );
    expect(second.result.current.isPending).toBe(true);
    act(() => second.result.current.mutate({}));
    expect(install).toHaveBeenCalledOnce();

    await act(async () => {
      resolveInstall?.({
        pluginId: "official/weather",
        outcome: { state: "installed" },
      });
    });
    await waitFor(() => expect(second.result.current.isPending).toBe(false));
  });
});
