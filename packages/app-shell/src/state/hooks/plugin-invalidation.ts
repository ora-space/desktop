import type { QueryClient } from "@tanstack/react-query";
import { queryKeys } from "./query-keys";

/**
 * Refreshes the installed and available plugin surfaces after any lifecycle change.
 * Shared by the import hook and per-plugin mutations so their invalidation cannot drift.
 */
export function invalidatePluginQueries(
  queryClient: QueryClient,
): Promise<void[]> {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.installedPlugins }),
    queryClient.invalidateQueries({ queryKey: queryKeys.availablePlugins }),
  ]);
}
