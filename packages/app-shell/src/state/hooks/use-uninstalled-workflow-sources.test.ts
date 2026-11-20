import { act, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type {
  InstalledPlugin,
  PluginConfigurationDetails,
  WorkflowSummary,
} from "@ora/contracts";
import { cachePluginConfiguration, pluginKeys } from "../data/plugins";
import { createTestClient } from "../../test/contracts-transport";
import { renderHookWithClient } from "../../test/hook-harness";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import {
  findUninstalledWorkflowSourceIds,
  useUninstalledWorkflowSources,
} from "./use-uninstalled-workflow-sources";

function workflowSummary(
  id: string,
  sourcePluginId: string | null,
): WorkflowSummary {
  return {
    id,
    namespace: "local",
    name: `Workflow ${id}`,
    publishedVersion: "1.0.0",
    sourcePluginId,
    createdAt: 1n,
    updatedAt: 1n,
  };
}

function installedWorkflowPlugin(id: string): InstalledPlugin {
  const slash = id.indexOf("/");
  return {
    id,
    namespace: id.slice(0, slash),
    name: id.slice(slash + 1),
    displayName: id,
    version: "1.0.0",
    description: "Workflow plugin",
    homepage: null,
    license: null,
    kind: "workflow",
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  };
}

describe("findUninstalledWorkflowSourceIds", () => {
  it("matches canonical <namespace>/<identifier> plugin ids and skips local workflows", () => {
    const missing = findUninstalledWorkflowSourceIds(
      [
        workflowSummary("wf-local", null),
        workflowSummary("wf-installed", "official/ora.workflows"),
        workflowSummary("wf-other-ns", "community/ora.workflows"),
        workflowSummary("wf-uninstalled", "official/ora.release"),
      ],
      [installedWorkflowPlugin("official/ora.workflows")],
    );

    expect([...missing]).toEqual(["wf-other-ns", "wf-uninstalled"]);
  });
});

describe("useUninstalledWorkflowSources", () => {
  it("skips querying installed plugins when no workflow carries a marketplace source", async () => {
    const listInstalledPlugins = vi.fn(async () => ({ plugins: [] }));
    const client = createTestClient({ listInstalledPlugins });
    const summaries = [workflowSummary("wf-local", null)];

    const { result } = renderHookWithClient(
      () => useUninstalledWorkflowSources(summaries),
      client,
    );

    expect(result.current.size).toBe(0);
    await Promise.resolve();
    expect(listInstalledPlugins).not.toHaveBeenCalled();
  });

  it("stays empty while installed plugins are pending or failing, and clears after reinstall", async () => {
    const state = createPluginMemory();
    const handlers = pluginHandlers(state);
    let resolveList:
      ((value: { plugins: InstalledPlugin[] }) => void) | undefined;
    const listSpy = vi
      .spyOn(handlers, "listInstalledPlugins")
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveList = resolve;
          }),
      );
    const client = createTestClient(handlers);
    const summaries = [
      workflowSummary("wf-market", "official/ora.cdase-workflow"),
      workflowSummary("wf-local", null),
    ];

    const { result, queryClient } = renderHookWithClient(
      () => useUninstalledWorkflowSources(summaries),
      client,
    );

    // Pending catalog must not briefly mark imported workflows as uninstalled.
    expect(result.current.size).toBe(0);

    await act(async () => {
      resolveList?.({ plugins: [] });
      await Promise.resolve();
    });
    await waitFor(() => expect([...result.current]).toEqual(["wf-market"]));

    // A failed refresh can retain the last successful (empty) catalog in the query cache.
    listSpy.mockRejectedValueOnce(new Error("catalog unavailable"));
    await act(async () => {
      await queryClient.invalidateQueries({
        queryKey: pluginKeys.installedPlugins,
      });
    });
    await waitFor(() =>
      expect(
        queryClient.getQueryState(pluginKeys.installedPlugins)?.status,
      ).toBe("error"),
    );
    await waitFor(() => expect(result.current.size).toBe(0));

    // Saving settings for one plugin cannot confirm which other plugins are installed.
    const configuration: PluginConfigurationDetails = {
      pluginId: "official/settings",
      schemaVersion: 1,
      revision: 1n,
      declarationFingerprint: "settings-v1",
      settings: [],
      summary: { state: "not_declared" },
    };
    act(() =>
      cachePluginConfiguration(
        queryClient,
        configuration.pluginId,
        configuration,
      ),
    );
    expect(
      queryClient.getQueryData(
        pluginKeys.pluginConfiguration(configuration.pluginId),
      ),
    ).toEqual(configuration);
    expect(queryClient.getQueryState(pluginKeys.installedPlugins)?.status).toBe(
      "error",
    );
    expect(result.current.size).toBe(0);

    // A successful refresh restores the absence marker, even if the catalog is still empty.
    await act(async () => {
      await queryClient.invalidateQueries({
        queryKey: pluginKeys.installedPlugins,
      });
    });
    await waitFor(() => expect([...result.current]).toEqual(["wf-market"]));

    // Reinstalling the exact canonical plugin id removes the marker.
    state.installedPlugins.push(
      installedWorkflowPlugin("official/ora.cdase-workflow"),
    );
    await act(async () => {
      await queryClient.invalidateQueries({
        queryKey: pluginKeys.installedPlugins,
      });
    });
    await waitFor(() => expect(result.current.size).toBe(0));
  });
});
