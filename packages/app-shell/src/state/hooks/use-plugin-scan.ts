import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { invalidateInstalledPlugins } from "../data/plugins";

/** Rescans installed packages and refreshes the installed-plugin query. */
export function usePluginScan() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => client.plugin.scan({}),
    onSettled: () => invalidateInstalledPlugins(queryClient),
  });
}
