import { useEffect, useMemo } from "react";
import type { Project, Session, Task } from "@ora/contracts";
import { useDraftSessionsStore } from "../stores/draft-sessions-store";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { resolveRestoredWorkspaceSelection } from "../resolve-restored-workspace-selection";
import { usePersistHydrated } from "./use-persist-hydrated";
import { useWorkflowRunsByProject } from "./use-workflow-runs";

/**
 * Applies a validated disk selection once the workspace tree has settled.
 *
 * Must not run before projects/tasks/sessions have **successfully** loaded:
 * treating a failed or still-empty fetch as "session gone" would clear
 * `pendingRestore` and persist that wipe, so the next launch has nothing left
 * to restore (intermittent cold-start misses).
 *
 * Draft candidates also wait for `draft-sessions-store` rehydration — otherwise
 * a still-empty in-memory draft list would look like "draft deleted" and clear
 * a perfectly valid restore.
 *
 * Selection-store hydration is required too: until `pendingRestore` is seeded
 * from disk, treating "null pending" as "nothing to restore" would skip the
 * saved session entirely.
 */
export function useRestoreWorkspaceSelection(input: {
  projects: readonly Project[];
  tasks: readonly Task[];
  sessions: readonly Session[];
  /**
   * True until every tree query has `isSuccess`. Callers must not pass mere
   * `isPending` — error/empty interim states must keep the candidate staged.
   */
  treePending: boolean;
}): void {
  const { projects, tasks, sessions, treePending } = input;
  const pendingRestore = useWorkspaceSelectionStore((s) => s.pendingRestore);
  const draftsHydrated = usePersistHydrated(useDraftSessionsStore.persist);
  const selectionHydrated = usePersistHydrated(
    useWorkspaceSelectionStore.persist,
  );

  const needsWorkflowRuns =
    pendingRestore !== null && pendingRestore.workflowRunId !== null;
  const workflowProjectId =
    needsWorkflowRuns && pendingRestore.projectId !== null
      ? pendingRestore.projectId
      : null;
  const runsQuery = useWorkflowRunsByProject(workflowProjectId);

  const workflowRuns = useMemo(() => {
    if (!needsWorkflowRuns) return [];
    // Sanitized candidates always carry a projectId with a run id. Without one
    // the by-project query stays disabled and would never leave pending.
    if (workflowProjectId === null) return [];
    // Mirror the tree gate: wait for success. An error/`data` miss must not
    // become `[]` → resolve miss → clearPendingRestore and wipe disk.
    if (!runsQuery.isSuccess) return null;
    return runsQuery.data ?? [];
  }, [
    needsWorkflowRuns,
    runsQuery.data,
    runsQuery.isSuccess,
    workflowProjectId,
  ]);

  useEffect(() => {
    if (
      treePending ||
      !draftsHydrated ||
      !selectionHydrated ||
      pendingRestore === null
    ) {
      return;
    }
    if (needsWorkflowRuns && workflowRuns === null) return;

    // Read drafts at apply time so keystroke updates do not re-trigger restore,
    // while still seeing the post-rehydrate list once `draftsHydrated` flips.
    const drafts = useDraftSessionsStore.getState().drafts;
    const resolved = resolveRestoredWorkspaceSelection({
      candidate: pendingRestore,
      projects,
      tasks,
      sessions,
      drafts,
      workflowRuns,
    });

    if (resolved.kind === "waiting") return;

    if (resolved.kind === "ready") {
      applyRestoredSelection(resolved.selection);
    } else {
      useWorkspaceSelectionStore.getState().clearPendingRestore();
    }
  }, [
    draftsHydrated,
    needsWorkflowRuns,
    pendingRestore,
    projects,
    selectionHydrated,
    sessions,
    tasks,
    treePending,
    workflowRuns,
  ]);
}

/** Commits a validated selection without touching expand/collapse state. */
function applyRestoredSelection(selection: {
  projectId: string | null;
  taskId: string | null;
  sessionId: string | null;
  workflowRunId: string | null;
  draftId: string | null;
}): void {
  const store = useWorkspaceSelectionStore.getState();
  if (selection.projectId === null) {
    store.clearPendingRestore();
    return;
  }

  // A tree click during restore only sets createFocus (selection stays empty).
  // Preserve that gesture so New chat still follows the row the user pointed at
  // instead of being overwritten when restore syncs focus from the restored leaf.
  const createFocusBefore = store.createFocus;
  store.commitRestoredSelection(selection);
  if (createFocusBefore !== null) {
    store.setCreateFocus(createFocusBefore);
  }
}
