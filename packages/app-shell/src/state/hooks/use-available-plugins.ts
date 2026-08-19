import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Loads the cached marketplace registry index surfaced by the backend. */
export function useAvailablePlugins() {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.availablePlugins,
    queryFn: () => client.plugin.listAvailable({}),
  });
}
