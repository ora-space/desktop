import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Loads the README one marketplace listing publishes for its detail page. */
export function usePluginReadme(pluginId: string) {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.pluginReadme(pluginId),
    queryFn: () => client.plugin.readReadme({ pluginId }),
  });
}
