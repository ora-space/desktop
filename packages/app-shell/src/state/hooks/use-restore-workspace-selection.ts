import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { Project, Session, Task } from "@ora/contracts";
import { useDraftSessionsStore } from "../stores/draft-sessions-store";
import { useUiStore } from "../stores/ui-store";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { isWorkspaceSelectionEmpty } from "../stores/sanitize-workspace-selection";
import { resolveRestoredWorkspaceSelection } from "../resolve-restored-workspace-selection";
import { useWorkflowRunsByProject } from "./use-workflow-runs";

/**
 * Subscribes to a zustand persist store's hydration so restore can wait for
 * disk drafts before treating a missing draft id as deleted.
 */
function usePersistHydrated(persistApi: {
  hasHydrated: () => boolean;
  onFinishHydration: (fn: (state: unknown) => void) => () => void;
}): boolean {
  return useSyncExternalStore(
    (onStoreChange) => persistApi.onFinishHydration(onStoreChange),
    () => persistApi.hasHydrated(),
    () => false,
  );
}

/**
 * Applies a validated disk selection once the workspace tree has settled.
 *
 * Must not run before projects/tasks/sessions finish their first fetch: putting
 * a session id into live selection while the sessions list is still empty would
 * look "unpersisted" to warm and open a stray provider session.
 *
 * Draft candidates also wait for `draft-sessions-store` rehydration — otherwise
 * a still-empty in-memory draft list would look like "draft deleted" and clear
 * a perfectly valid restore.
 */
export function useRestoreWorkspaceSelection(input: {
  projects: readonly Project[];
  tasks: readonly Task[];
  sessions: readonly Session[];
  /** True while any of the three tree queries is still pending or has errored. */
  treePending: boolean;
}): void {
  const { projects, tasks, sessions, treePending } = input;
  const pendingRestore = useWorkspaceSelectionStore((s) => s.pendingRestore);
  const draftsHydrated = usePersistHydrated(useDraftSessionsStore.persist);

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
    // Treat an errored run list like a still-pending one: return null so the
    // effect keeps waiting instead of resolving against an empty error list,
    // which would discard a valid restore candidate as a miss. A later refetch
    // that succeeds re-runs the effect with the real list.
    if (runsQuery.isPending || runsQuery.isError) return null;
    return runsQuery.data ?? [];
  }, [
    needsWorkflowRuns,
    runsQuery.data,
    runsQuery.isError,
    runsQuery.isPending,
    workflowProjectId,
  ]);

  useEffect(() => {
    if (treePending || pendingRestore === null || !draftsHydrated) return;
    if (needsWorkflowRuns && workflowRuns === null) return;

    const live = useWorkspaceSelectionStore.getState().selection;
    if (!isWorkspaceSelectionEmpty(live)) {
      // User already navigated; keep their choice and stop retrying disk.
      useWorkspaceSelectionStore.getState().clearPendingRestore();
      return;
    }

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
    sessions,
    tasks,
    treePending,
    workflowRuns,
  ]);
}

/** Routes a validated selection through the store APIs and expands ancestors. */
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
  // instead of being overwritten when select* syncs focus from the restored leaf.
  const createFocusBefore = store.createFocus;

  if (selection.sessionId !== null && selection.taskId !== null) {
    store.selectSession(
      selection.sessionId,
      selection.taskId,
      selection.projectId,
    );
  } else if (selection.draftId !== null) {
    store.selectDraft(selection.draftId, selection.taskId, selection.projectId);
  } else if (selection.workflowRunId !== null) {
    store.selectWorkflowRun(selection.workflowRunId, selection.projectId);
  } else if (selection.taskId !== null) {
    store.selectTask(selection.taskId, selection.projectId);
  } else {
    store.selectProject(selection.projectId);
  }

  if (createFocusBefore !== null) {
    store.setCreateFocus(createFocusBefore);
  }

  useUiStore.getState().expandProject(selection.projectId);
  if (selection.taskId !== null) {
    useUiStore.getState().expandTask(selection.taskId);
  }
}
