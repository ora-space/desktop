import { useMemo } from "react";
import type { InstalledPlugin, WorkflowSummary } from "@ora/contracts";
import { useInstalledPlugins } from "./use-installed-plugins";

const EMPTY_WORKFLOW_IDS: ReadonlySet<string> = new Set();

/**
 * Returns the workflow ids whose marketplace source plugin is no longer installed.
 *
 * Matching uses the full canonical `<namespace>/<identifier>` plugin id so an identically named
 * package from another namespace never clears the marker, while workflows without a marketplace
 * source record stay unmarked.
 */
export function findUninstalledWorkflowSourceIds(
  workflows: readonly WorkflowSummary[],
  installedPlugins: readonly InstalledPlugin[],
): ReadonlySet<string> {
  const installedIds = new Set(installedPlugins.map((plugin) => plugin.id));
  const missing = new Set<string>();
  for (const workflow of workflows) {
    if (
      workflow.sourcePluginId !== null &&
      !installedIds.has(workflow.sourcePluginId)
    ) {
      missing.add(workflow.id);
    }
  }
  return missing;
}

/**
 * Identifies workflows whose marketplace source plugin is currently absent from `installedPlugins`.
 *
 * Only queries installed plugins when at least one workflow carries a marketplace source, and
 * stays empty while installed plugins are loading or unavailable so transient catalog errors never
 * mark every imported workflow as missing its source.
 */
export function useUninstalledWorkflowSources(
  workflows: readonly WorkflowSummary[] | undefined,
): ReadonlySet<string> {
  const hasSourced = (workflows ?? []).some(
    (workflow) => workflow.sourcePluginId !== null,
  );
  const installed = useInstalledPlugins(hasSourced);

  return useMemo(() => {
    if (
      !hasSourced ||
      workflows === undefined ||
      installed.data === undefined
    ) {
      return EMPTY_WORKFLOW_IDS;
    }
    return findUninstalledWorkflowSourceIds(workflows, installed.data);
  }, [hasSourced, workflows, installed.data]);
}
