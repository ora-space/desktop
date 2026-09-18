import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { invalidatePluginQueries } from "../data/plugins";

/**
 * Imports one local `.orax` plugin archive selected by the user and refreshes
 * the installed and available surfaces once the backend settles.
 */
export function usePluginImport() {
  const client = useContractsClient();
  const queryClient = useQueryClient();

  return useMutation({
    // The archive's kind is only known after the import reads it, so this flow cannot disclose a
    // Hook's execution before asking for it. A Hook that arrives this way lands uninitialized and
    // the user authorizes it from the installed list, which keeps the authorization tied to the
    // one Hook it applies to.
    mutationFn: ({ path }: { path: string }) =>
      client.plugin.import({ path, hookExecutionAcknowledged: false }),
    onSettled: () => invalidatePluginQueries(queryClient),
  });
}
