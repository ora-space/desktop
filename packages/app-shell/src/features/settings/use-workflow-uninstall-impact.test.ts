import { QueryClient } from "@tanstack/react-query";
import { waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { InstalledPlugin } from "@ora/contracts";
import { createTestClient } from "../../test/contracts-transport";
import { renderHookWithClient } from "../../test/hook-harness";
import { pluginKeys } from "../../state/data/plugins";
import { skillKeys } from "../../state/data/skills";
import { useWorkflowUninstallImpact } from "./use-workflow-uninstall-impact";

/** A Skill plugin whose catalog entry was removed, so it owns no workflow-bindable identity. */
const skillPlugin: InstalledPlugin = {
  id: "official/release-pack",
  namespace: "official",
  name: "release-pack",
  displayName: "Release Pack",
  version: "1.0.0",
  description: "Release skills",
  homepage: null,
  license: null,
  kind: "skill",
  logo: null,
  installationValidity: { validity: "valid" },
  configuration: { state: "not_declared" },
  runtime: "stopped",
};

/** Mirrors the app's cache policy: freshly loaded catalogs stay fresh, so nothing refetches. */
function createWarmQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: 30_000 },
      mutations: { retry: false },
    },
  });
}

describe("useWorkflowUninstallImpact", () => {
  it("skips the workflow library when the catalogs resolve no bindable target", async () => {
    const listWorkflows = vi.fn(async () => ({ workflows: [] }));
    const client = createTestClient({ listWorkflows });
    const queryClient = createWarmQueryClient();
    queryClient.setQueryData(pluginKeys.installedPlugins, [skillPlugin]);
    queryClient.setQueryData(skillKeys.skills, []);

    const { result } = renderHookWithClient(
      () => useWorkflowUninstallImpact([skillPlugin.id], true),
      client,
      queryClient,
    );

    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.impact).toEqual({ dependents: [], unreadable: 0 });
    // No workflow can reference this plugin, so even the library listing is wasted work.
    expect(listWorkflows).not.toHaveBeenCalled();
  });

  it("queries workflow summaries without fetching drafts or skills for a workflow plugin", async () => {
    const workflowPlugin: InstalledPlugin = {
      ...skillPlugin,
      id: "official/ora.cdase-workflow",
      name: "ora.cdase-workflow",
      displayName: "CDASE Workflow",
      kind: "workflow",
      configuration: { state: "not_declared" },
    };
    const getWorkflow = vi.fn(async () => {
      throw new Error("workflow plugin check must not fetch individual drafts");
    });
    const listSkills = vi.fn(async () => {
      throw new Error("unrelated skills catalog error");
    });
    const client = createTestClient({
      listWorkflows: async () => ({
        workflows: [
          {
            id: "wf-1",
            namespace: "local",
            name: "导入流程",
            publishedVersion: "1.0.0",
            sourcePluginId: workflowPlugin.id,
            createdAt: 1n,
            updatedAt: 1n,
          },
          {
            id: "wf-local",
            namespace: "local",
            name: "本地流程",
            publishedVersion: null,
            sourcePluginId: null,
            createdAt: 2n,
            updatedAt: 2n,
          },
        ],
      }),
      getWorkflow,
      listSkills,
    });
    const queryClient = createWarmQueryClient();
    queryClient.setQueryData(pluginKeys.installedPlugins, [workflowPlugin]);

    const { result } = renderHookWithClient(
      () => useWorkflowUninstallImpact([workflowPlugin.id], true),
      client,
      queryClient,
    );

    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.impact).toEqual({ dependents: [], unreadable: 0 });
    expect(result.current.importedWorkflows).toEqual([
      { workflowId: "wf-1", name: "导入流程" },
    ]);
    expect(getWorkflow).not.toHaveBeenCalled();
    expect(listSkills).not.toHaveBeenCalled();
  });

  it("fails the check instead of reporting no imports when the workflow library cannot be read", async () => {
    const workflowPlugin: InstalledPlugin = {
      ...skillPlugin,
      id: "official/ora.cdase-workflow",
      name: "ora.cdase-workflow",
      displayName: "CDASE Workflow",
      kind: "workflow",
    };
    const client = createTestClient({
      listWorkflows: async () => {
        throw new Error("workflow library unavailable");
      },
    });
    const queryClient = createWarmQueryClient();
    queryClient.setQueryData(pluginKeys.installedPlugins, [workflowPlugin]);

    const { result } = renderHookWithClient(
      () => useWorkflowUninstallImpact([workflowPlugin.id], true),
      client,
      queryClient,
    );

    await waitFor(() => expect(result.current.status).toBe("failed"));
    expect(result.current.checking).toBe(false);
    // The unreadable library must be surfaced for the retained-copy list, not silently
    // treated as an empty set of imported workflows.
    expect(result.current).toMatchObject({
      status: "failed",
      dependencyImpactUnknown: false,
      retentionUnknown: true,
    });
  });
});
