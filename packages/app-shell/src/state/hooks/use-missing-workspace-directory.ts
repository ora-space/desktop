import type { Workspace } from "@ora/contracts";
import { useWorkspaces } from "./use-workspaces";

/** Returns the resolved path when this workspace's local folder is gone, otherwise null. */
export function missingWorkspaceDirectory(
  workspace: Workspace | undefined,
): string | null {
  return workspace?.directory.kind === "missing"
    ? workspace.directory.path
    : null;
}

/**
 * Reads the backend's last probe of one workspace folder from the shared workspace list.
 *
 * Null covers both "present" and "not known yet": callers gate on a confirmed missing folder
 * only, so a list still in flight never blocks a healthy project.
 */
export function useMissingWorkspaceDirectory(
  workspaceId: string | null | undefined,
): string | null {
  const { data: workspaces } = useWorkspaces();
  if (workspaceId === null || workspaceId === undefined) return null;
  return missingWorkspaceDirectory(
    workspaces?.find((workspace) => workspace.id === workspaceId),
  );
}
