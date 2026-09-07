import { waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { createTestClient } from "../../test/contracts-transport";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { renderHookWithClient } from "../../test/hook-harness";
import { pluginKeys } from "../data/plugins";
import { useAvailablePlugins } from "./use-available-plugins";

describe("useAvailablePlugins", () => {
  it("loads the cached registry catalog through the contracts client", async () => {
    const state = createPluginMemory();
    state.availablePlugins.push({
      id: "official/weather",
      name: "weather",
      title: "Weather",
      kind: "workbench",
      namespace: "official",
      sourceUrl: "https://github.com/ora-space/marketplace",
      version: "1.2.0",
      description: "Weather plugin",
      logo: null,
      compatibility: "compatible",
    });
    const { result, queryClient } = renderHookWithClient(
      () => useAvailablePlugins(),
      createTestClient(pluginHandlers(state)),
    );

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual({
      updatedAt: 0n,
      plugins: state.availablePlugins,
    });
    expect(queryClient.getQueryData(pluginKeys.availablePlugins)).toEqual({
      updatedAt: 0n,
      plugins: state.availablePlugins,
    });
  });
});
