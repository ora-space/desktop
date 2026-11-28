import { useCallback, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { pluginKeys } from "../data/plugins";

/** Loads the backend's immutable startup snapshot of installed plugin manifests. */
export function useInstalledPlugins(enabled = true) {
  const client = useContractsClient();
  return useQuery({
    queryKey: pluginKeys.installedPlugins,
    queryFn: () =>
      client.plugin.listInstalled({}).then((response) => response.plugins),
    enabled,
  });
}

/**
 * Resolves a Canonical Plugin ID to the display name of the installed plugin.
 *
 * Until the installed snapshot loads, or for a plugin it does not list, the name segment of the
 * id stands in: an origin tooltip naming the package identifier is still accurate.
 */
export function usePluginDisplayName(): (pluginId: string) => string {
  const installed = useInstalledPlugins();
  const names = useMemo(
    () =>
      new Map(
        (installed.data ?? []).map((plugin) => [plugin.id, plugin.displayName]),
      ),
    [installed.data],
  );
  return useCallback(
    (pluginId: string) =>
      names.get(pluginId) ?? pluginId.slice(pluginId.indexOf("/") + 1),
    [names],
  );
}
