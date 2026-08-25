import type {
  Project,
  Session,
  Task,
  WorkflowRunSummary,
  Workspace,
} from "@ora/contracts";
import type { SessionDraft } from "./stores/draft-sessions-store";
import type { WorkspaceSelection } from "./stores/sanitize-workspace-selection";
import { isWorkspaceSelectionEmpty } from "./stores/sanitize-workspace-selection";

export interface ResolveRestoredWorkspaceSelectionInput {
  candidate: WorkspaceSelection;
  projects: readonly Project[];
  tasks: readonly Task[];
  sessions: readonly Session[];
  /** Loaded Workspace rows let direct sessions derive their project without a Task. */
  workspaces?: readonly Workspace[];
  drafts: readonly SessionDraft[];
  /**
   * Runs for `candidate.projectId` when restoring a workflow run. Pass `null`
   * when the run list has not settled yet so the caller can wait instead of
   * treating a missing list as "run deleted".
   */
  workflowRuns: readonly WorkflowRunSummary[] | null;
}

export type ResolvedWorkspaceSelection =
  | { kind: "ready"; selection: WorkspaceSelection }
  | { kind: "waiting" }
  | { kind: "miss" };

/**
 * Validates a disk restore candidate against the loaded workspace tree.
 *
 * `ready` carries a selection for `select*` APIs. `waiting` means a workflow
 * run list is still loading. `miss` means the target no longer exists (or the
 * payload was empty). Ownership always comes from the authoritative
 * session/task/run records, so a mismatched persisted projectId cannot point
 * chat at the wrong project.
 */
export function resolveRestoredWorkspaceSelection(
  input: ResolveRestoredWorkspaceSelectionInput,
): ResolvedWorkspaceSelection {
  const {
    candidate,
    projects,
    tasks,
    sessions,
    workspaces = [],
    drafts,
    workflowRuns,
  } = input;
  if (isWorkspaceSelectionEmpty(candidate)) return { kind: "miss" };

  if (candidate.sessionId !== null) {
    const session = sessions.find((item) => item.id === candidate.sessionId);
    if (session === undefined) return { kind: "miss" };
    const workspace = workspaces.find(
      (item) => item.id === session.workspaceId,
    );
    const workspaceTask = tasks.find(
      (item) => item.workspaceId === session.workspaceId,
    );
    if (workspace === undefined && workspaceTask === undefined) {
      return { kind: "miss" };
    }
    const projectId = workspace?.projectId ?? workspaceTask?.projectId;
    if (
      projectId === undefined ||
      !projects.some((item) => item.id === projectId)
    ) {
      return { kind: "miss" };
    }
    return {
      kind: "ready",
      selection: {
        projectId,
        taskId: workspaceTask?.id ?? null,
        sessionId: session.id,
        workflowRunId: null,
        draftId: null,
      },
    };
  }

  if (candidate.draftId !== null) {
    const draft = drafts.find((item) => item.id === candidate.draftId);
    if (draft === undefined) return { kind: "miss" };
    if (!projects.some((item) => item.id === draft.projectId)) {
      return { kind: "miss" };
    }
    if (
      draft.taskId !== null &&
      !tasks.some(
        (item) =>
          item.id === draft.taskId && item.projectId === draft.projectId,
      )
    ) {
      return { kind: "miss" };
    }
    return {
      kind: "ready",
      selection: {
        projectId: draft.projectId,
        taskId: draft.taskId,
        sessionId: null,
        workflowRunId: null,
        draftId: draft.id,
      },
    };
  }

  if (candidate.workflowRunId !== null) {
    if (candidate.projectId === null) return { kind: "miss" };
    if (!projects.some((item) => item.id === candidate.projectId)) {
      return { kind: "miss" };
    }
    if (workflowRuns === null) return { kind: "waiting" };
    // Re-derive ownership from the authoritative run record instead of the
    // persisted candidate, matching the session/draft/task branches. The run
    // list is project-scoped today, but a future caller passing an unscoped
    // list must not let a corrupt candidate.projectId retarget the run.
    const run = workflowRuns.find(
      (item) =>
        item.id === candidate.workflowRunId &&
        item.projectId === candidate.projectId,
    );
    if (run === undefined) return { kind: "miss" };
    return {
      kind: "ready",
      selection: {
        projectId: run.projectId,
        taskId: null,
        sessionId: null,
        workflowRunId: run.id,
        draftId: null,
      },
    };
  }

  if (candidate.taskId !== null) {
    const task = tasks.find((item) => item.id === candidate.taskId);
    if (task === undefined) return { kind: "miss" };
    if (!projects.some((item) => item.id === task.projectId)) {
      return { kind: "miss" };
    }
    return {
      kind: "ready",
      selection: {
        projectId: task.projectId,
        taskId: task.id,
        sessionId: null,
        workflowRunId: null,
        draftId: null,
      },
    };
  }

  if (candidate.projectId !== null) {
    if (!projects.some((item) => item.id === candidate.projectId)) {
      return { kind: "miss" };
    }
    return {
      kind: "ready",
      selection: {
        projectId: candidate.projectId,
        taskId: null,
        sessionId: null,
        workflowRunId: null,
        draftId: null,
      },
    };
  }

  return { kind: "miss" };
}
