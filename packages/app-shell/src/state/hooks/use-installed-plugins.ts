import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { pluginKeys } from "../data/plugins";

/** Loads the backend's immutable startup snapshot of installed plugin manifests. */
export function useInstalledPlugins() {
  const client = useContractsClient();
  return useQuery({
    queryKey: pluginKeys.installedPlugins,
    queryFn: () =>
      client.plugin.listInstalled({}).then((response) => response.plugins),
  });
}
