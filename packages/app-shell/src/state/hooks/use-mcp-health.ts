import type { McpHealthEntry } from "@ora/contracts";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { invalidateMcpHealth, mcpHealthKeys } from "../data/mcp-health";

/**
 * Loads Host MCP health for the plugin card (`null`) or one Session workspace (`cwd`).
 *
 * An unresolved view (`undefined`, or a workspace whose directory is still empty) leaves the query
 * disabled: answering with the card view would be a different question, and the card view is
 * exactly where a workspace-context member must stay `context_missing`.
 */
export function useMcpHealth(cwd: string | null | undefined) {
  const client = useContractsClient();
  const view = cwd === undefined || cwd === "" ? undefined : cwd;
  return useQuery({
    queryKey: mcpHealthKeys.view(view ?? null),
    enabled: view !== undefined,
    queryFn: () =>
      client.plugin
        .listMcpHealth({ cwd: view ?? null })
        .then((response) => response.entries),
  });
}

/**
 * Runs one user-initiated Host MCP re-detect and adopts the authoritative result.
 *
 * Waiting is allowed here because completing the probe is exactly what the request asked for; the
 * probe still has its own hard timeout. The view is invalidated afterwards so the refreshed rows
 * come from the same secret-free query as every other read.
 */
export function useProbeMcpHealth(pluginId: string, cwd: string | null) {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => client.plugin.probeMcpHealth({ pluginId, cwd }),
    onSuccess: (response) => {
      const entry: McpHealthEntry = response.entry;
      queryClient.setQueryData(
        mcpHealthKeys.view(cwd),
        (previous: McpHealthEntry[] | undefined) =>
          previous === undefined
            ? previous
            : previous.map((candidate) =>
                candidate.identity.pluginId === entry.identity.pluginId &&
                candidate.identity.cwd === entry.identity.cwd
                  ? entry
                  : candidate,
              ),
      );
      return invalidateMcpHealth(queryClient);
    },
  });
}
