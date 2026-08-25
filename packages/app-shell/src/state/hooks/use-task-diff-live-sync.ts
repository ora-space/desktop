import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { ChatStore, ChatToolCall, SessionConversation } from "@ora/chat";
import type { Session, Task } from "@ora/contracts";
import { queryKeys } from "./query-keys";

const DIFF_REFRESH_DEBOUNCE_MS = 400;

interface ConversationDiffSnapshot {
  completedFileChanges: Set<string>;
  isResponding: boolean;
}

/**
 * Invalidates task diff snapshots after each completed file-change item and
 * once more when its turn ends.
 */
export function useTaskDiffLiveSync(
  chatStore: ChatStore,
  sessions: Session[],
  tasks: readonly Task[] = [],
): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    const taskByWorkspace = new Map(
      tasks.map((task) => [task.workspaceId, task.id]),
    );
    const taskBySession = new Map(
      sessions.map((session) => [
        session.id,
        taskByWorkspace.get(session.workspaceId),
      ]),
    );
    const snapshots = new Map<string, ConversationDiffSnapshot>();
    const pendingTaskIds = new Set<string>();
    let refreshTimer: ReturnType<typeof setTimeout> | null = null;

    for (const [sessionId, conversation] of Object.entries(
      chatStore.getState().conversations,
    )) {
      snapshots.set(sessionId, conversationDiffSnapshot(conversation));
    }

    // A short debounce collapses agents that finish several file edits back-to-back
    // into one Git diff computation while retaining operation-level feedback.
    const scheduleRefresh = (taskId: string) => {
      pendingTaskIds.add(taskId);
      if (refreshTimer !== null) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => {
        refreshTimer = null;
        const taskIds = [...pendingTaskIds];
        pendingTaskIds.clear();
        for (const pendingTaskId of taskIds) {
          void queryClient.invalidateQueries({
            queryKey: queryKeys.taskDiffs(pendingTaskId),
          });
        }
      }, DIFF_REFRESH_DEBOUNCE_MS);
    };

    const unsubscribe = chatStore.subscribe((state) => {
      for (const [sessionId, taskId] of taskBySession) {
        if (taskId === undefined) continue;
        const conversation = state.conversations[sessionId];
        if (conversation === undefined) continue;

        const previous = snapshots.get(sessionId);
        const next = conversationDiffSnapshot(conversation);
        snapshots.set(sessionId, next);
        if (previous === undefined) continue;

        const completedFileChange = [...next.completedFileChanges].some(
          (toolCallId) => !previous.completedFileChanges.has(toolCallId),
        );
        const turnCompleted = previous.isResponding && !next.isResponding;

        // Replayed history may introduce old completed tools in one batch. Only a
        // live turn can promote a newly observed tool into a diff refresh event.
        if (
          (completedFileChange &&
            (previous.isResponding || next.isResponding)) ||
          turnCompleted
        ) {
          scheduleRefresh(taskId);
        }
      }
    });

    return () => {
      unsubscribe();
      if (refreshTimer !== null) clearTimeout(refreshTimer);
    };
  }, [chatStore, queryClient, sessions, tasks]);
}

/** Captures only the lifecycle facts that can advance the aggregate task diff. */
function conversationDiffSnapshot(
  conversation: SessionConversation,
): ConversationDiffSnapshot {
  return {
    completedFileChanges: new Set(
      conversation.turns.flatMap((turn) =>
        turn.items
          .filter(
            (item): item is ChatToolCall =>
              item.kind === "toolCall" &&
              item.status === "completed" &&
              isFileChange(item),
          )
          .map((item) => `${turn.id}:${item.id}`),
      ),
    ),
    isResponding: conversation.isResponding,
  };
}

/** Recognizes ACP file-change items without depending on agent-specific tool titles. */
function isFileChange(toolCall: ChatToolCall): boolean {
  return (
    toolCall.toolKind === "edit" ||
    toolCall.toolKind === "delete" ||
    toolCall.toolKind === "move" ||
    toolCall.content.some((content) => content.type === "diff")
  );
}
