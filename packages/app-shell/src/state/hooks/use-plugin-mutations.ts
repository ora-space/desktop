import type { InstalledPlugin, PluginDataDisposition } from "@ora/contracts";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { usePluginOperationStore } from "../stores/plugin-operation-store";
import { queryKeys } from "./query-keys";
import { invalidatePluginQueries } from "./plugin-invalidation";

/** Provides lifecycle mutations for one installed plugin and invalidates the plugin queries on settle. */
export function usePluginMutations(pluginId: string, agentRef?: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const activity = usePluginOperationStore(
    (state) => state.activities[pluginId],
  );
  const invalidate = () => invalidatePluginQueries(queryClient);
  const refreshAgent = (agentRef: string, scope: "availability" | "models") => {
    // Every lifecycle change invalidates availability and its display cache.
    // Only a starting agent can answer model discovery; stopping one must not
    // retry model discovery against a runtime just removed.
    const requests = [
      queryClient.invalidateQueries({ queryKey: queryKeys.agentRuntimeStatus }),
    ];
    if (scope === "models") {
      requests.push(
        queryClient.invalidateQueries({
          queryKey: queryKeys.agentModelsForAgent(agentRef),
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
      ? refreshAgent(plugin.id, scope)
      : Promise.resolve([]);

  const activate = useMutation({
    mutationFn: () => client.plugin.activate({ pluginId }),
    onSuccess: ({ plugin }) => refreshPluginAgent(plugin, "models"),
    onSettled: async () => {
      try {
        await invalidate();
      } finally {
        usePluginOperationStore.getState().clear(pluginId);
      }
    },
  });
  const stop = useMutation({
    mutationFn: () => client.plugin.stop({ pluginId }),
    onSuccess: ({ plugin }) => refreshPluginAgent(plugin, "availability"),
    onSettled: async () => {
      try {
        await invalidate();
      } finally {
        usePluginOperationStore.getState().clear(pluginId);
      }
    },
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
    onSettled: async () => {
      try {
        await invalidate();
      } finally {
        usePluginOperationStore.getState().clear(pluginId);
      }
    },
  });

  const activateMutate = (...args: Parameters<typeof activate.mutate>) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "activate")) return;
    activate.mutate(...args);
  };
  const activateMutateAsync = (
    ...args: Parameters<typeof activate.mutateAsync>
  ) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "activate")) {
      return Promise.reject(
        new Error(`plugin operation already pending: ${pluginId}`),
      );
    }
    return activate.mutateAsync(...args);
  };
  const stopMutate = (...args: Parameters<typeof stop.mutate>) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "stop")) return;
    stop.mutate(...args);
  };
  const stopMutateAsync = (...args: Parameters<typeof stop.mutateAsync>) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "stop")) {
      return Promise.reject(
        new Error(`plugin operation already pending: ${pluginId}`),
      );
    }
    return stop.mutateAsync(...args);
  };
  const uninstallMutate = (...args: Parameters<typeof uninstall.mutate>) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "uninstall"))
      return;
    uninstall.mutate(...args);
  };
  const uninstallMutateAsync = (
    ...args: Parameters<typeof uninstall.mutateAsync>
  ) => {
    if (!usePluginOperationStore.getState().begin(pluginId, "uninstall")) {
      return Promise.reject(
        new Error(`plugin operation already pending: ${pluginId}`),
      );
    }
    return uninstall.mutateAsync(...args);
  };

  return {
    activate: {
      ...activate,
      isPending: activity?.state === "pending" && activity.kind === "activate",
      mutate: activateMutate,
      mutateAsync: activateMutateAsync,
    },
    stop: {
      ...stop,
      isPending: activity?.state === "pending" && activity.kind === "stop",
      mutate: stopMutate,
      mutateAsync: stopMutateAsync,
    },
    uninstall: {
      ...uninstall,
      isPending: activity?.state === "pending" && activity.kind === "uninstall",
      mutate: uninstallMutate,
      mutateAsync: uninstallMutateAsync,
    },
  };
}
