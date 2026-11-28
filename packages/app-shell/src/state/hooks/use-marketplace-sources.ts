import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  MarketplaceArtifactRetrievalUpdate,
  UpdateMarketplaceSourceRequest,
} from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { invalidateMarketplaceSourceChange } from "../data/plugin-lifecycle";
import { pluginKeys } from "../data/plugins";

/** Loads the user-configured marketplace source list. */
export function useMarketplaceSources() {
  const client = useContractsClient();
  return useQuery({
    queryKey: pluginKeys.marketplaceSources,
    queryFn: () => client.plugin.listSources({}),
  });
}

/**
 * Adds one marketplace source and refreshes the source list, and every origin
 * label the new source can change, after the backend persists it.
 *
 * The add contract always stores Direct HTTPS. S3 credentials stay on the
 * existing update path, so choosing SigV4 follows the add with that update.
 * If the credential write fails, the new row is removed: the dialog submits
 * once and does not leave a source whose retrieval differs from the form.
 */
export function useAddMarketplaceSource() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      url,
      branch,
      useProxy,
      artifactRetrieval,
    }: {
      url: string;
      branch: string;
      useProxy: boolean;
      artifactRetrieval: MarketplaceArtifactRetrievalUpdate;
    }) => {
      await client.plugin.addSource({ url, branch, useProxy });
      if (artifactRetrieval.type === "direct_https") return;
      try {
        await client.plugin.updateSource({
          url,
          newUrl: url,
          branch,
          useProxy,
          enabled: true,
          artifactRetrieval,
        });
      } catch (error) {
        try {
          await client.plugin.deleteSource({ url });
        } catch (rollbackError) {
          // The dialog reports the credential failure. A failed rollback leaves
          // the Direct HTTPS row, which the list refresh below makes visible.
          void rollbackError;
        }
        throw error;
      }
    },
    onSettled: () => invalidateMarketplaceSourceChange(queryClient),
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
    onSettled: () => invalidateMarketplaceSourceChange(queryClient),
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
    mutationFn: (request: UpdateMarketplaceSourceRequest) =>
      client.plugin.updateSource(request),
    onSettled: () => invalidateMarketplaceSourceChange(queryClient),
  });
}
