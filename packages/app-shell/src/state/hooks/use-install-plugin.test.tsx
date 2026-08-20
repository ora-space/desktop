import { act, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { useInstallPlugin } from "./use-install-plugin";

describe("useInstallPlugin", () => {
  it("installs a marketplace plugin and refreshes the installed surface", async () => {
    const state = createMockClientState();
    state.availablePlugins.push({
      id: "official/weather",
      name: "weather",
      namespace: "official",
      version: "1.2.0",
      description: "Weather",
    });
    const client = createMockClient(state);
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
      packageName: "official/weather",
      displayName: "weather",
      version: "1.2.0",
    });
  });
});
