import type {
  InstalledPlugin,
  PluginConfigurationDetails,
} from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";

/**
 * Refreshes the installed and available plugin surfaces after any lifecycle change.
 * Shared by the import hook and per-plugin mutations so their invalidation cannot drift.
 */
export function invalidatePluginQueries(
  queryClient: QueryClient,
): Promise<void[]> {
  return Promise.all([
    invalidateInstalledPlugins(queryClient),
    invalidateAvailablePlugins(queryClient),
  ]);
}

/** Cache identity owned by plugins data; consumers never repeat its tuples. */
export const pluginKeys = {
  availablePlugins: ["available-plugins"] as const,
  pluginReadme: (pluginId: string) => ["plugin-readme", pluginId] as const,
  marketplaceSources: ["marketplace-sources"] as const,
  installedPlugins: ["installed-plugins"] as const,
  pluginConfiguration: (pluginId: string) =>
    ["plugin-configuration", pluginId] as const,
};

/** Refreshes installed state after a scan without forcing a marketplace fetch. */
export function invalidateInstalledPlugins(queryClient: QueryClient) {
  return queryClient.invalidateQueries({
    queryKey: pluginKeys.installedPlugins,
  });
}

/** Refreshes marketplace results after a registry sync. */
export function invalidateAvailablePlugins(queryClient: QueryClient) {
  return queryClient.invalidateQueries({
    queryKey: pluginKeys.availablePlugins,
  });
}

/** Refreshes source configuration; registry sync remains an explicit operation. */
export function invalidateMarketplaceSources(queryClient: QueryClient) {
  return queryClient.invalidateQueries({
    queryKey: pluginKeys.marketplaceSources,
  });
}

/** Adopts one confirmed configuration into both detail and installed-list projections. */
export function cachePluginConfiguration(
  queryClient: QueryClient,
  pluginId: string,
  configuration: PluginConfigurationDetails,
): void {
  queryClient.setQueryData(
    pluginKeys.pluginConfiguration(pluginId),
    configuration,
  );
  queryClient.setQueryData<InstalledPlugin[]>(
    pluginKeys.installedPlugins,
    (plugins) =>
      plugins?.map((plugin) =>
        plugin.id === pluginId
          ? { ...plugin, configuration: configuration.summary }
          : plugin,
      ),
  );
}
