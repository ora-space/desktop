import { waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { queryKeys } from "./query-keys";
import { useInstalledPlugins } from "./use-installed-plugins";

describe("useInstalledPlugins", () => {
  it("loads the cached installed plugin list through the contracts client", async () => {
    const state = createMockClientState();
    state.installedPlugins.push({
      id: "ora.reviewer",
      packageName: "@ora-plugins/reviewer",
      displayName: "Reviewer",
      version: "0.1.0",
      kind: "agent",
      main: "dist/index.js",
      agent: { displayName: "Reviewer", contractVersion: 1 },
      enabled: false,
      runtime: "stopped",
    });
    const { result, queryClient } = renderHookWithClient(
      () => useInstalledPlugins(),
      createMockClient(state),
    );

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual(state.installedPlugins);
    expect(queryClient.getQueryData(queryKeys.installedPlugins)).toEqual(
      state.installedPlugins,
    );
  });
});
