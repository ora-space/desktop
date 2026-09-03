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
    mutationFn: ({
      url,
      branch,
      useProxy,
    }: {
      url: string;
      branch: string;
      useProxy: boolean;
    }) => client.plugin.addSource({ url, branch, useProxy }),
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

/**
 * Changes one marketplace source's URL, branch, proxy policy, or enabled state
 * and refreshes the list.
 */
export function useUpdateMarketplaceSource() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      url,
      newUrl,
      branch,
      useProxy,
      enabled,
    }: {
      url: string;
      newUrl: string;
      branch: string;
      useProxy: boolean;
      enabled: boolean;
    }) =>
      client.plugin.updateSource({
        url,
        newUrl,
        branch,
        useProxy,
        enabled,
      }),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: queryKeys.marketplaceSources }),
  });
}
