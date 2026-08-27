import { act, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { useUpdatePlugin } from "./use-update-plugin";

describe("useUpdatePlugin", () => {
  it("updates an installed plugin and refreshes the installed surface", async () => {
    const state = createMockClientState();
    state.availablePlugins.push({
      id: "official/weather",
      name: "weather",
      title: "Weather",
      kind: "agent",
      namespace: "official",
      version: "1.1.0",
      description: "Weather",
      logo: null,
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
    const client = createMockClient(state);
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
});
