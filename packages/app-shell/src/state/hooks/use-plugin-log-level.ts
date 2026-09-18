import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { RuntimeLogLevel } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { pluginKeys } from "../data/plugins";

/** Loads and updates one plugin's host-owned log level through the shared contracts client. */
export function usePluginLogLevel(pluginId: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: pluginKeys.pluginLogLevel(pluginId),
    queryFn: () => client.plugin.getLogLevel({ pluginId }),
  });
  const mutation = useMutation({
    mutationFn: (level: RuntimeLogLevel) =>
      client.plugin.setLogLevel({ pluginId, level }),
    // Only a backend response may replace the cache: a failed persist means the host kept the
    // previous level, so the menu must keep showing it rather than the requested one.
    onSuccess: (response) => {
      queryClient.setQueryData(pluginKeys.pluginLogLevel(pluginId), response);
    },
  });

  return {
    state: query.data,
    isLoading: query.isPending,
    isSaving: mutation.isPending,
    setLevel: mutation.mutate,
  };
}
