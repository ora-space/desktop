import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Rescans installed packages and refreshes the installed-plugin query. */
export function usePluginScan() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => client.plugin.scan({}),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.installedPlugins }),
  });
}
