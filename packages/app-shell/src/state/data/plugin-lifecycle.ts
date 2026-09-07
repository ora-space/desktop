import type { InstalledPlugin } from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";
import { invalidateAgentAvailability, refreshAgent } from "./agent-runtime";
import { invalidateInstalledPlugins } from "./plugins";
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
