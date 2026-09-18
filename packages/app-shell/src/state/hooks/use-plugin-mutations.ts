import type { PluginDataDisposition } from "@ora/contracts";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { usePluginOperationStore } from "../stores/plugin-operation-store";
import {
  invalidateAgentAvailability,
  refreshAgent,
} from "../data/agent-runtime";
import { refreshPluginAgent } from "../data/plugin-lifecycle";
import { invalidatePluginQueries } from "../data/plugins";

/** What one removal request carries beyond the plugin it names. */
interface UninstallPluginVariables {
  /** Whether the package's own data directory is removed with the package. */
  dataDisposition: PluginDataDisposition;
  /**
   * Declares that the user authorized running the package's Hook `deinit` command.
   *
   * Unlike the install and update acknowledgements this one is part of a confirmation the user
   * was going to see anyway, so the dialogs that disclose what removal runs can declare it while
   * every other caller leaves the package's program untouched.
   */
  hookExecutionAcknowledged?: boolean;
}

/** Provides lifecycle mutations for one installed plugin and invalidates the plugin queries on settle. */
export function usePluginMutations(pluginId: string, agentRef?: string) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const activity = usePluginOperationStore(
    (state) => state.activities[pluginId],
  );
  const invalidate = () => invalidatePluginQueries(queryClient);
  const activate = useMutation({
    mutationFn: () => client.plugin.activate({ pluginId }),
    onSuccess: ({ plugin }) =>
      refreshPluginAgent(queryClient, plugin, "models"),
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
    onSuccess: ({ plugin }) =>
      refreshPluginAgent(queryClient, plugin, "availability"),
    onSettled: async () => {
      try {
        await invalidate();
      } finally {
        usePluginOperationStore.getState().clear(pluginId);
      }
    },
  });
  const uninstall = useMutation({
    mutationFn: ({
      dataDisposition,
      hookExecutionAcknowledged = false,
    }: UninstallPluginVariables) =>
      client.plugin.uninstall({
        pluginId,
        dataDisposition,
        hookExecutionAcknowledged,
      }),
    // Unlike the other lifecycle endpoints, uninstall returns only the plugin
    // id. Callers that still own the installed snapshot provide its package
    // identity so agent availability and display caches cannot survive removal.
    onSuccess: () =>
      agentRef === undefined
        ? invalidateAgentAvailability(queryClient)
        : refreshAgent(queryClient, agentRef, "availability"),
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
