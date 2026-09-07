import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { agentKeys } from "../data/agents";

/** Loads configurable agents through the contracts client and caches them. */
export function useAgents() {
  const client = useContractsClient();
  return useQuery({
    queryKey: agentKeys.agents,
    queryFn: () => client.agent.list({}).then((response) => response.agents),
  });
}
