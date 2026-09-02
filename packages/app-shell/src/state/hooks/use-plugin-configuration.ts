import type {
  InstalledPlugin,
  PluginConfigurationDetails,
  ResetPluginConfigurationRequest,
  SavePluginConfigurationRequest,
} from "@ora/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

type ResetConfigurationInput =
  ResetPluginConfigurationRequest extends infer Request
    ? Request extends unknown
      ? Omit<Request, "pluginId">
      : never
    : never;

/** Loads and mutates one Plugin Configuration while keeping list and detail caches coherent. */
export function usePluginConfiguration(pluginId: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.pluginConfiguration(pluginId),
    queryFn: () =>
      client.plugin
        .getConfiguration({ pluginId })
        .then((response) => response.configuration),
    enabled: pluginId !== "",
  });

  const adopt = (configuration: PluginConfigurationDetails) => {
    queryClient.setQueryData(
      queryKeys.pluginConfiguration(pluginId),
      configuration,
    );
    queryClient.setQueryData<InstalledPlugin[]>(
      queryKeys.installedPlugins,
      (plugins) =>
        plugins?.map((plugin) =>
          plugin.id === pluginId
            ? { ...plugin, configuration: configuration.summary }
            : plugin,
        ),
    );
  };
  const save = useMutation({
    mutationFn: (request: Omit<SavePluginConfigurationRequest, "pluginId">) =>
      client.plugin.saveConfiguration({ pluginId, ...request }),
    onSuccess: (response) => adopt(response.configuration),
  });
  const reset = useMutation({
    mutationFn: (request: ResetConfigurationInput) =>
      client.plugin.resetConfiguration({ pluginId, ...request }),
    onSuccess: (response) => adopt(response.configuration),
  });

  return { query, save, reset };
}
