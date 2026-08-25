import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createElement, type ReactNode } from "react";
import {
  createChatStore,
  type ChatStore,
  type SessionConversation,
} from "@ora/chat";
import type { Session, Task } from "@ora/contracts";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { queryKeys } from "./query-keys";
import { useTaskDiffLiveSync } from "./use-task-diff-live-sync";

const SESSION: Session = {
  id: "session-1",
  workspaceId: "workspace-1",
  agentRef: "ora-space.codeagentcli",
  status: "running",
  title: null,
  historyState: { type: "writable" },
};
const TASK: Task = {
  id: "task-1",
  projectId: "project-1",
  workspaceId: "workspace-1",
  title: "Task",
};

/** Builds one conversation state with just enough lifecycle data for diff syncing. */
function conversation(
  isResponding: boolean,
  toolStatus?: "in_progress" | "completed" | "failed",
): SessionConversation {
  return {
    configOptions: [],
    modelChanges: [],
    historyNotices: [],
    turns:
      toolStatus === undefined
        ? []
        : [
            {
              id: "turn-1",
              userMessage: {
                kind: "message",
                id: "message-1",
                role: "user",
                content: "change it",
                createdAt: 1,
              },
              items: [
                {
                  kind: "toolCall",
                  id: "tool-1",
                  title: "Edit file",
                  toolKind: "edit",
                  status: toolStatus,
                  content: [],
                  locations: [],
                  createdAt: 2,
                  updatedAt: 3,
                },
              ],
              status: isResponding ? "streaming" : "completed",
              stopReason: null,
              error: null,
              createdAt: 1,
            },
          ],
    availableCommands: [],
    isLoaded: true,
    isLoading: false,
    isResponding,
    sessionTitle: null,
    sessionUpdatedAt: null,
    pendingPermissions: [],
    error: null,
  };
}

/** Creates an isolated chat store whose state tests can advance directly. */
function makeChatStore(): ChatStore {
  return createChatStore(createMockClient(createMockClientState()).session);
}

/** Provides the query cache observed by the live-sync hook. */
function wrapper(queryClient: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return createElement(
      QueryClientProvider,
      { client: queryClient },
      children,
    );
  };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("useTaskDiffLiveSync", () => {
  it("invalidates the aggregate task diff after a live file change completes", async () => {
    const chatStore = makeChatStore();
    chatStore.setState({
      conversations: { [SESSION.id]: conversation(true, "in_progress") },
    });
    const queryClient = new QueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    renderHook(() => useTaskDiffLiveSync(chatStore, [SESSION], [TASK]), {
      wrapper: wrapper(queryClient),
    });

    act(() => {
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(true, "completed") },
      });
      vi.advanceTimersByTime(400);
    });

    expect(invalidate).toHaveBeenCalledOnce();
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: queryKeys.taskDiffs(TASK.id),
    });
  });

  it("coalesces the file-change and turn-completed refreshes", () => {
    const chatStore = makeChatStore();
    chatStore.setState({
      conversations: { [SESSION.id]: conversation(true, "in_progress") },
    });
    const queryClient = new QueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    renderHook(() => useTaskDiffLiveSync(chatStore, [SESSION], [TASK]), {
      wrapper: wrapper(queryClient),
    });

    act(() => {
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(true, "completed") },
      });
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(false, "completed") },
      });
      vi.advanceTimersByTime(400);
    });

    expect(invalidate).toHaveBeenCalledOnce();
  });

  it("does not treat replayed completed tools as live changes", () => {
    const chatStore = makeChatStore();
    const queryClient = new QueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    renderHook(() => useTaskDiffLiveSync(chatStore, [SESSION], [TASK]), {
      wrapper: wrapper(queryClient),
    });

    act(() => {
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(false, "completed") },
      });
      vi.advanceTimersByTime(400);
    });

    expect(invalidate).not.toHaveBeenCalled();
  });
});
