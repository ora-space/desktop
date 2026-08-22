import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";
import { WORKSPACE_LIST_NOTIFY_PROPS } from "./workspace-list-query";

/** Loads the visible agent session list through the contracts client and caches it. */
export function useSessions() {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.sessions,
    queryFn: () =>
      client.session.list({}).then((response) => response.sessions),
    notifyOnChangeProps: [...WORKSPACE_LIST_NOTIFY_PROPS],
  });
}
