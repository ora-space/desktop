import type { InstalledPlugin, PluginDataDisposition } from "@ora/contracts";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { useAgentModelStore } from "../stores/agent-model-store";
import { queryKeys } from "./query-keys";
import { invalidatePluginQueries } from "./plugin-invalidation";

/** Provides lifecycle mutations for one installed plugin and invalidates the plugin queries on settle. */
export function usePluginMutations(pluginId: string, agentRef?: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const invalidate = () => invalidatePluginQueries(queryClient);
  const refreshAgent = (agentRef: string, scope: "availability" | "models") => {
    // Every lifecycle change invalidates availability and its display cache.
    // Only a starting agent can answer model discovery; stopping one must not
    // retry the permanently pinned warm query against a runtime just removed.
    useAgentModelStore.getState().forget(agentRef);
    const requests = [
      queryClient.invalidateQueries({ queryKey: queryKeys.agentRuntimeStatus }),
    ];
    if (scope === "models") {
      requests.push(
        queryClient.invalidateQueries({
          queryKey: queryKeys.warmSessionsForAgent(agentRef),
        }),
      );
    }
    return Promise.all(requests);
  };
  const refreshPluginAgent = (
    plugin: InstalledPlugin,
    scope: "availability" | "models",
  ) =>
    plugin.kind === "agent"
      ? refreshAgent(plugin.name, scope)
      : Promise.resolve([]);

  const enable = useMutation({
    mutationFn: () => client.plugin.enable({ pluginId }),
    onSuccess: ({ plugin }) => refreshPluginAgent(plugin, "models"),
    onSettled: invalidate,
  });
  const disable = useMutation({
    mutationFn: () => client.plugin.disable({ pluginId }),
    onSuccess: ({ plugin }) => refreshPluginAgent(plugin, "availability"),
    onSettled: invalidate,
  });
  const activate = useMutation({
    mutationFn: () => client.plugin.activate({ pluginId }),
    onSuccess: ({ plugin }) => refreshPluginAgent(plugin, "models"),
    onSettled: invalidate,
  });
  const stop = useMutation({
    mutationFn: () => client.plugin.stop({ pluginId }),
    onSuccess: ({ plugin }) => refreshPluginAgent(plugin, "availability"),
    onSettled: invalidate,
  });
  const uninstall = useMutation({
    mutationFn: (dataDisposition?: PluginDataDisposition) =>
      client.plugin.uninstall({
        pluginId,
        dataDisposition: dataDisposition ?? "delete",
      }),
    // Unlike the other lifecycle endpoints, uninstall returns only the plugin
    // id. Callers that still own the installed snapshot provide its package
    // identity so agent availability and display caches cannot survive removal.
    onSuccess: () =>
      agentRef === undefined
        ? queryClient.invalidateQueries({
            queryKey: queryKeys.agentRuntimeStatus,
          })
        : refreshAgent(agentRef, "availability"),
    onSettled: invalidate,
  });

  return { enable, disable, activate, stop, uninstall };
}
