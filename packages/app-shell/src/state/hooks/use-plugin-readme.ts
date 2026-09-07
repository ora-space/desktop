import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { pluginKeys } from "../data/plugins";

/** Loads the README one marketplace listing publishes for its detail page. */
export function usePluginReadme(pluginId: string) {
  const client = useContractsClient();
  return useQuery({
    queryKey: pluginKeys.pluginReadme(pluginId),
    queryFn: () => client.plugin.readReadme({ pluginId }),
  });
}
