import { waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { queryKeys } from "./query-keys";
import { useAvailablePlugins } from "./use-available-plugins";

describe("useAvailablePlugins", () => {
  it("loads the cached registry catalog through the contracts client", async () => {
    const state = createMockClientState();
    state.availablePlugins.push({
      id: "official/weather",
      name: "weather",
      namespace: "official",
      version: "1.2.0",
      description: "Weather plugin",
    });
    const { result, queryClient } = renderHookWithClient(
      () => useAvailablePlugins(),
      createMockClient(state),
    );

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual({
      updatedAt: 0n,
      plugins: state.availablePlugins,
    });
    expect(queryClient.getQueryData(queryKeys.availablePlugins)).toEqual({
      updatedAt: 0n,
      plugins: state.availablePlugins,
    });
  });
});
