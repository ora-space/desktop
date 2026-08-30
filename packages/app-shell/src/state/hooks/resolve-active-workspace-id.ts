/**
 * Derives the single workspace id the app is currently pointed at, or `null`
 * when no chat workspace applies.
 *
 * Precedence mirrors the warm-session resolver in `use-warm-session.ts`: a graph
 * workflow run has no Agent chat, so it yields no workspace; otherwise an isolated
 * task worktree wins over the project's main checkout, and a missing task or
 * workspace reads as `null` while its list is still loading rather than as a
 * different surface.
 */
export function resolveActiveWorkspaceId(
  selection: {
    projectId: string | null;
    taskId: string | null;
    workflowRunId?: string | null;
  },
  tasks: readonly { id: string; workspaceId: string }[],
  workspaces: readonly {
    id: string;
    projectId: string;
    kind: "main" | "isolated";
  }[],
): string | null {
  if (
    selection.workflowRunId !== null &&
    selection.workflowRunId !== undefined
  ) {
    return null;
  }
  if (selection.taskId !== null) {
    return (
      tasks.find((task) => task.id === selection.taskId)?.workspaceId ?? null
    );
  }
  if (selection.projectId !== null) {
    return (
      workspaces.find(
        (workspace) =>
          workspace.projectId === selection.projectId &&
          workspace.kind === "main",
      )?.id ?? null
    );
  }
  return null;
}
