import { useMemo } from "react";
import { useAgentDiscovery } from "../../state/hooks/use-agent-discovery";
import { usePendingSwitch } from "../../state/stores/pending-agent-store";
import { type DiscoveredCommands, discoveredCommand } from "./agent-commands";

/**
 * Stands in for a session's own command report where there is none to show.
 *
 * A report, even an empty one, always wins for the agent that made it. While a move onto another
 * agent is pending it describes the agent being left, so the incoming agent's commands are shown.
 */
export function useDiscoveredCommands({
  sessionId,
  hasReport,
  isLoadingHistory,
  agentRef,
  workspaceId,
  directoryMissing,
}: {
  sessionId: string | null;
  hasReport: boolean;
  /** A report is replayed with the history, so until that lands "none yet" is not "none". */
  isLoadingHistory: boolean;
  /** The agent the next message goes to. */
  agentRef: string | null;
  workspaceId: string | undefined;
  directoryMissing: boolean;
}): DiscoveredCommands | undefined {
  const pendingSwitch = usePendingSwitch(sessionId);
  const isNeeded =
    agentRef !== null &&
    (pendingSwitch !== undefined || (!hasReport && !isLoadingHistory));
  const discovery = useAgentDiscovery(
    isNeeded && !directoryMissing ? agentRef : null,
    workspaceId ?? null,
  );
  const { commands, isLoading } = discovery;
  return useMemo(
    () =>
      isNeeded
        ? {
            agentRef,
            commands: commands?.map(discoveredCommand) ?? null,
            isLoading,
          }
        : undefined,
    [agentRef, commands, isLoading, isNeeded],
  );
}
