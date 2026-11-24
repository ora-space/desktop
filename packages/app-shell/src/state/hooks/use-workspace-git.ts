import { useMemo } from "react";
import type { Workspace } from "@ora/contracts";
import { useWorkspaces } from "./use-workspaces";

/**
 * Whether Git-only features (Changes, worktree tasks) apply to one workspace.
 *
 * - `checking`: the shared workspace list has not loaded yet.
 * - `notRepository`: the backend's last probe found a present folder outside any Git
 *   repository, so Git features must explain that instead of failing.
 * - `available`: everything else, including a confirmed repository, a probe the backend could
 *   not answer, a missing or remote folder, and a failed list. Those keep the previous
 *   behavior so an inconclusive probe never hides a working feature.
 */
export type WorkspaceGitAvailability =
  "checking" | "notRepository" | "available";

/** Reads the Git availability the backend reported for one listed workspace. */
function workspaceGitAvailability(
  workspace: Workspace | undefined,
): Exclude<WorkspaceGitAvailability, "checking"> {
  return workspace?.directory.kind === "present" &&
    workspace.directory.git === "notRepository"
    ? "notRepository"
    : "available";
}

/**
 * Selects the workspace that decides availability: an exact workspace id when one is given,
 * otherwise the main workspace of the project (the folder a worktree task would branch from).
 */
type WorkspaceGitTarget =
  | { workspaceId: string | null | undefined }
  | { projectId: string | null | undefined };

/**
 * Derives Git availability from the shared workspace list, which already refetches on window
 * focus and on workspace events, so a folder that gains a repository later is picked up then.
 */
export function useWorkspaceGitAvailability(
  target: WorkspaceGitTarget,
): WorkspaceGitAvailability {
  const { data: workspaces, isPending } = useWorkspaces();
  if (isPending) return "checking";
  const workspace =
    "workspaceId" in target
      ? workspaces?.find((candidate) => candidate.id === target.workspaceId)
      : workspaces?.find(
          (candidate) =>
            candidate.projectId === target.projectId &&
            candidate.kind === "main",
        );
  return workspaceGitAvailability(workspace);
}

/**
 * Lists the projects whose main folder the backend last reported as `notRepository`.
 *
 * For lists that act on many projects at once (the scheduled-task switches), where calling the
 * per-target hook per row is not possible. A list still loading or failed yields no project, so
 * nothing is blocked on an inconclusive answer.
 */
export function useProjectsWithoutGit(): ReadonlySet<string> {
  const { data: workspaces } = useWorkspaces();
  return useMemo(
    () =>
      new Set(
        (workspaces ?? [])
          .filter(
            (workspace) =>
              workspace.kind === "main" &&
              workspaceGitAvailability(workspace) === "notRepository",
          )
          .map((workspace) => workspace.projectId),
      ),
    [workspaces],
  );
}
