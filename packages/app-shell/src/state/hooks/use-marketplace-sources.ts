import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Loads the user-configured marketplace source list. */
export function useMarketplaceSources() {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.marketplaceSources,
    queryFn: () => client.plugin.listSources({}),
  });
}

/**
 * Adds one marketplace source and refreshes the source list after the backend
 * persists it.
 */
export function useAddMarketplaceSource() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ url, branch }: { url: string; branch: string }) =>
      client.plugin.addSource({ url, branch }),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.marketplaceSources }),
  });
}

/**
 * Removes one marketplace source by URL and refreshes the persisted list once
 * the backend settles.
 */
export function useDeleteMarketplaceSource() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ url }: { url: string }) =>
      client.plugin.deleteSource({ url }),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.marketplaceSources }),
  });
}
