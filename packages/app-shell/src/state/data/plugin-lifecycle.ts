import type { InstalledPlugin } from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";
import { invalidateAgentAvailability, refreshAgent } from "./agent-runtime";
import {
  invalidateAvailablePlugins,
  invalidateInstalledPlugins,
  invalidateMarketplaceSources,
} from "./plugins";
import { invalidateSkills } from "./skills";

/** Coordinates only the runtime projections owned by an agent package. */
export function refreshPluginAgent(
  queryClient: QueryClient,
  plugin: InstalledPlugin,
  scope: "availability" | "models",
) {
  return plugin.kind === "agent"
    ? refreshAgent(queryClient, plugin.id, scope)
    : Promise.resolve([]);
}

/** External package transitions affect installed projections, not the marketplace catalog. */
export function invalidatePluginState(queryClient: QueryClient): void {
  void invalidateInstalledPlugins(queryClient);
  void invalidateAgentAvailability(queryClient);
  void invalidateSkills(queryClient);
}

/**
 * Refreshes everything a marketplace source change can relabel.
 *
 * Adding a source can record a namesake binding, which changes how other sources' origins are
 * labelled in Skill lists, the installed list, and the marketplace. The host computes labels per
 * query, so refetching is all it takes for every surface to show the disambiguated labels.
 */
export function invalidateMarketplaceSourceChange(
  queryClient: QueryClient,
): Promise<unknown> {
  return Promise.all([
    invalidateMarketplaceSources(queryClient),
    invalidateSkills(queryClient),
    invalidateInstalledPlugins(queryClient),
    invalidateAvailablePlugins(queryClient),
  ]);
}
