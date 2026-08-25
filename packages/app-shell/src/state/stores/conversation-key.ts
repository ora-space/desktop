/**
 * Names the conversation a selection is looking at, for frontend-only state that
 * belongs to one conversation rather than one task.
 *
 * A live session keys on its own id, so sibling sessions under the same task stay
 * independent. A client-only draft keys on its own id so two parked composers
 * under the same Workspace do not share plugin picks. A selection with no
 * session or draft yet falls back to its isolated task or direct project
 * Workspace, and is rekeyed onto the real session id once the send mints one.
 */
export function conversationKeyFor(selection: {
  projectId?: string | null;
  sessionId: string | null;
  taskId: string | null;
  draftId: string | null;
}): string {
  if (selection.sessionId !== null) return selection.sessionId;
  if (selection.draftId !== null) return `draft:${selection.draftId}`;
  if (selection.taskId !== null) return `task:${selection.taskId}`;
  if (selection.projectId !== null && selection.projectId !== undefined) {
    return `project:${selection.projectId}`;
  }
  return "__none__";
}
