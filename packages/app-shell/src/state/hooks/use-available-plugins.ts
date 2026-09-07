import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { pluginKeys } from "../data/plugins";

/** Loads the cached marketplace registry index surfaced by the backend. */
export function useAvailablePlugins() {
  const client = useContractsClient();
  return useQuery({
    queryKey: pluginKeys.availablePlugins,
    queryFn: () => client.plugin.listAvailable({}),
  });
}
