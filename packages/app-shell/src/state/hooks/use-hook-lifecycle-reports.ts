import type { HookLifecycleReport } from "@ora/contracts";
import { useQuery } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { pluginKeys } from "../data/plugins";

/**
 * Reads this session's Hook lifecycle results, keyed by plugin id.
 *
 * A Hook the host has never executed for has no result at all, which is how the installed list
 * distinguishes "never initialized" from "initialized and failed" without a state of its own. The
 * query is only enabled for a Hook row: packages of other kinds cannot produce a result, and
 * asking for them would fire a request no surface reads.
 */
export function useHookLifecycleReports(enabled: boolean) {
  const client = useContractsClient();
  return useQuery({
    queryKey: pluginKeys.hookLifecycleReports,
    queryFn: () => client.plugin.listHookLifecycleReports({}),
    enabled,
    select: (response) =>
      new Map<string, HookLifecycleReport>(
        response.reports.map((report) => [report.pluginId, report]),
      ),
  });
}
