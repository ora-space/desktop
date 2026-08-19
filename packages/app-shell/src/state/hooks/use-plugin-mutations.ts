import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Provides lifecycle mutations for one installed plugin and invalidates the plugin queries on settle. */
export function usePluginMutations(pluginId: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const invalidate = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.installedPlugins }),
      queryClient.invalidateQueries({ queryKey: queryKeys.availablePlugins }),
    ]);

  const enable = useMutation({
    mutationFn: () => client.plugin.enable({ pluginId }),
    onSettled: invalidate,
  });
  const disable = useMutation({
    mutationFn: () => client.plugin.disable({ pluginId }),
    onSettled: invalidate,
  });
  const activate = useMutation({
    mutationFn: () => client.plugin.activate({ pluginId }),
    onSettled: invalidate,
  });
  const stop = useMutation({
    mutationFn: () => client.plugin.stop({ pluginId }),
    onSettled: invalidate,
  });
  const uninstall = useMutation({
    mutationFn: () => client.plugin.uninstall({ pluginId }),
    onSettled: invalidate,
  });

  return { enable, disable, activate, stop, uninstall };
}
