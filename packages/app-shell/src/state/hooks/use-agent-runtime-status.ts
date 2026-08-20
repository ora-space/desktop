import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Short-poll cadence used only while at least one CLI is still completing its ACP handshake. */
const STARTING_POLL_INTERVAL_MS = 1500;

/**
 * Loads the live per-CLI detection status (ready/starting/unavailable/failing) used to drive the
 * plugin settings pane's installed indicator. Polls at a short interval only while some CLI
 * is still starting; once every CLI has settled into ready, unavailable, or failing, polling stops.
 */
export function useAgentRuntimeStatus() {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.agentRuntimeStatus,
    queryFn: () =>
      client.agentRuntime.getStatus({}).then((response) => response.statuses),
    refetchInterval: (query) =>
      query.state.data?.some((status) => status.status === "starting")
        ? STARTING_POLL_INTERVAL_MS
        : false,
  });
}
