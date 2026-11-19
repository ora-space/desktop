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
});
