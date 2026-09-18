import { act, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
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
import { workflowKeys } from "../data/workflows";
import { usePluginImport } from "./use-plugin-import";

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(): TestHandlers {
  const state = createPluginMemory();
  // A concrete target makes the import commit a package instead of rejecting it.
  state.importTarget = state.installedPlugins[0];
  return { ...pluginHandlers(state) };
}

describe("usePluginImport", () => {
  it("refreshes the workflow library alongside the plugin surfaces", async () => {
    const client = createTestClient(createFixtureHandlers());
    const queryClient = createTestQueryClient();
    // Seed the library so invalidation has a query to mark; an unseeded key has no state.
    queryClient.setQueryData(workflowKeys.library, []);
    expect(queryClient.getQueryState(workflowKeys.library)?.isInvalidated).toBe(
      false,
    );
    const { result } = renderHookWithClient(
      () => usePluginImport(),
      client,
      queryClient,
    );

    act(() => result.current.mutate({ path: "C:/downloads/workflows.orax" }));

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    await waitFor(() =>
      expect(
        queryClient.getQueryState(workflowKeys.library)?.isInvalidated,
      ).toBe(true),
    );
  });
});
