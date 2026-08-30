import { useQuery } from "@tanstack/react-query";
import type { McpApplicationStateDto } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { useTasks } from "./use-tasks";
import { useWorkspaces } from "./use-workspaces";
import { resolveActiveWorkspaceId } from "./resolve-active-workspace-id";
import { queryKeys } from "./query-keys";

/**
 * How often to re-read while the surface is still converging on its own.
 *
 * `waiting_for_agent` resolves once the OpenCode agent starts, and `applying`
 * once the reconcile loop finishes writing and activating — neither needs this
 * client to act, only to notice.
 */
const TRANSIENT_MCP_STATE_POLL_INTERVAL_MS = 2000;

/** True for the states the reconcile loop moves through without this client doing anything. */
function isTransientMcpState(
  state: McpApplicationStateDto | undefined,
): boolean {
  return state === "waiting_for_agent" || state === "applying";
}

/** Read-only view of the MCP Application State for one workspace's OpenCode surface. */
export interface McpApplicationStateController {
  /** The workspace whose MCP surface is being read, or `null` when no chat workspace is active. */
  workspaceId: string | null;
  /** The folded state, or `undefined` while the first read is still in flight. */
  state: McpApplicationStateDto | undefined;
  /** True only while the first read is in flight; a later refetch keeps the prior state on screen. */
  isLoading: boolean;
  /** The last read failure, or `null`. Cleared by a successful refetch. */
  error: Error | null;
  /** Re-reads the state, used by the retry affordance shown after a failure. */
  refetch: () => void;
}

/**
 * Loads the user-visible MCP Application State for the active workspace's OpenCode surface.
 *
 * The active workspace follows the global tree selection (an isolated task worktree wins over the
 * project's main checkout; a graph workflow run has no Agent chat and yields nothing), so one panel
 * tracks whatever surface the user is currently looking at. Polling runs only while the state is
 * still expected to change on its own — a resting state stops — and the panel is inert while the
 * dialog hosting it is closed.
 */
export function useMcpApplicationState(
  enabled = true,
): McpApplicationStateController {
  const client = useContractsClient();
  const selection = useWorkspaceSelectionStore((state) => state.selection);
  const { data: tasks = [] } = useTasks();
  const { data: workspaces = [] } = useWorkspaces();
  const workspaceId = resolveActiveWorkspaceId(selection, tasks, workspaces);
  const mcpQuery = useQuery({
    queryKey: queryKeys.mcpApplicationState(workspaceId ?? ""),
    queryFn: () =>
      client.effect.getMcpApplicationState({ workspaceId: workspaceId! }),
    enabled: enabled && workspaceId !== null,
    refetchInterval: (query) =>
      isTransientMcpState(query.state.data?.state)
        ? TRANSIENT_MCP_STATE_POLL_INTERVAL_MS
        : false,
  });
  return {
    workspaceId,
    state: mcpQuery.data?.state,
    isLoading: mcpQuery.isLoading,
    error: mcpQuery.error,
    refetch: () => {
      void mcpQuery.refetch();
    },
  };
}
